#!/usr/bin/env python3
"""Exercise real terminal input, resize, and cleanup using disposable PTYs."""
import errno
import fcntl
import os
import pathlib
import pty
import json
import select
import signal
import struct
import subprocess
import sys
import termios
import tempfile
import time
import unittest

BINARY = str(pathlib.Path(sys.argv.pop(1) if len(sys.argv) > 1 else "target/debug/top-dog").resolve())


class Session:
    def __init__(self, config=None):
        self.master, self.slave = pty.openpty()
        self.attributes = termios.tcgetattr(self.slave)
        self.resize(100, 24)

        def controlling_terminal():
            os.setsid()
            fcntl.ioctl(self.slave, termios.TIOCSCTTY, 0)

        command = [BINARY, "--refresh-ms", "100", "--pid", str(os.getpid())]
        if config:
            command.extend(["--config", str(config)])
        self.child = subprocess.Popen(
            command,
            stdin=self.slave, stdout=self.slave, stderr=self.slave,
            preexec_fn=controlling_terminal, env={**os.environ, "TERM": "xterm-256color"},
        )
        self.output = b""

    def resize(self, width, height):
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))

    def send(self, data):
        self.output = b""
        os.write(self.master, data)

    def expect(self, text):
        deadline = time.monotonic() + 5
        while text.encode() not in self.output:
            if time.monotonic() > deadline:
                raise AssertionError(f"terminal did not display {text!r}; tail={self.output[-1800:]!r}")
            if select.select([self.master], [], [], 0.1)[0]:
                try:
                    self.output = (self.output + os.read(self.master, 65536))[-131072:]
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
            if self.child.poll() is not None:
                raise AssertionError("terminal process exited before expected output")

    def exited_cleanly(self):
        assert self.child.wait(timeout=5) == 0
        assert termios.tcgetattr(self.slave) == self.attributes, "terminal attributes were not restored"

    def close(self):
        if self.child.poll() is None:
            self.child.kill()
            self.child.wait()
        os.close(self.master)
        os.close(self.slave)


class TerminalTests(unittest.TestCase):
    def setUp(self):
        self.session = Session()
        self.addCleanup(self.session.close)
        self.session.expect("TOP DOG")
        # The header renders before the collector supplies a selectable process.
        self.session.expect("\x1b[7m")

    def test_filter_help_confirmation_resize_and_quit(self):
        s = self.session
        s.send(b"?")
        s.expect("TOP DOG controls")
        s.send(b"\x1b")
        s.expect("TOP DOG  up")
        s.send(b"/unique-no-process-match-987\r")
        s.expect("No matching processes")
        s.send(b"\x1b")
        s.expect("filter ''  PID all")
        s.send(b"s\r")
        s.expect("Press y to confirm")
        s.send(b"n")
        s.expect("Action cancelled")
        s.resize(50, 12)
        os.kill(s.child.pid, signal.SIGWINCH)
        # Wait for the footer at its resized position before sending more input.
        s.expect("\x1b[12;1H\x1b[0mEnter inspect")
        s.send(b"?")
        s.expect("TOP DOG controls")
        s.send(b"\x1b")
        s.expect("TOP DOG  up")
        s.send(b"q")
        s.exited_cleanly()

    def test_ctrl_c_restores_terminal(self):
        self.session.send(b"\x03")
        self.session.exited_cleanly()

    def test_sigterm_restores_terminal(self):
        os.kill(self.session.child.pid, signal.SIGTERM)
        self.session.exited_cleanly()

    def plugin_session(self, count):
        directory = tempfile.TemporaryDirectory(prefix="top-dog-terminal-")
        self.addCleanup(directory.cleanup)
        root = pathlib.Path(directory.name)
        marker = root / "activated"
        fixture = root / "plugin.py"
        fixture.write_text('''import json,sys,pathlib
r=json.load(sys.stdin)
if r['method']=='resolve':
    targets=[{'id':str(i),'title':'Fixture tab '+str(i),'url':None,'window':'1','relationship':'application','can_activate':True} for i in range(int(sys.argv[2]))]
    data={'kind':'resolution','value':{'process':r['params']['process'],'application':'Fixture','summary':'Related fixture tabs','targets':targets}}
else:
    pathlib.Path(sys.argv[1]).write_text(r['params']['target_id'])
    data={'kind':'activation','value':{'selected':True,'focus':'requested','message':'Fixture selection complete'}}
print(json.dumps({'version':1,'id':r['id'],'status':'ok','data':data}))
''')
        config = root / "config.toml"
        config.write_text(
            '[[plugins]]\nname="fixture"\nprotocol_version=1\n'
            + 'executable=' + json.dumps(sys.executable) + '\n'
            + 'args=' + json.dumps([str(fixture), str(marker), str(count)]) + '\n'
            + 'match_executables=' + json.dumps([pathlib.Path(os.readlink('/proc/self/exe')).name]) + '\n'
        )
        session = Session(config)
        self.addCleanup(session.close)
        session.expect("python")
        return session, marker

    def test_shared_process_chooser_activates_the_selected_target(self):
        session, marker = self.plugin_session(2)
        session.send(b"o")
        session.expect("Fixture targets")
        session.send(b"\x1b[B\r")
        session.expect("Fixture selection complete")
        self.assertEqual(marker.read_text(), "1")
        session.send(b"q")
        session.exited_cleanly()

    def test_single_target_opens_directly(self):
        session, marker = self.plugin_session(1)
        session.send(b"o")
        session.expect("Fixture selection complete")
        self.assertEqual(marker.read_text(), "0")
        session.send(b"q")
        session.exited_cleanly()


if __name__ == "__main__":
    unittest.main()
