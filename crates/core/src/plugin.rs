//! One bounded exchange per subprocess. Each request is independent, including activation.
use crate::config::PluginConfig;
use std::{
    io::{Read, Write},
    os::{fd::AsRawFd, unix::process::CommandExt},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};
use top_dog_protocol::{MAX_MESSAGE_BYTES, Operation, Outcome, Reply, Request, Response, VERSION};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

struct RequestProcess {
    child: Child,
    group: i32,
}
impl Drop for RequestProcess {
    fn drop(&mut self) {
        // This group was created for one request and contains no monitor processes.
        unsafe {
            libc::kill(-self.group, libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn nonblocking(pipe: &impl AsRawFd) -> Result<(), String> {
    // fcntl changes flags on our owned pipe descriptor only.
    let flags = unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_GETFL) };
    if flags < 0
        || unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

fn read_step(pipe: &mut impl Read, data: &mut Vec<u8>, limit: usize) -> Result<bool, String> {
    let mut buffer = [0; 8192];
    match pipe.read(&mut buffer) {
        Ok(0) => Ok(true),
        Ok(count) => {
            if data.len() + count > limit {
                return Err("plugin output exceeds message limit".into());
            }
            data.extend_from_slice(&buffer[..count]);
            Ok(false)
        }
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
            ) =>
        {
            Ok(false)
        }
        Err(e) => Err(e.to_string()),
    }
}

pub fn call(
    plugin: &PluginConfig,
    operation: Operation,
    timeout: Duration,
) -> Result<Reply, String> {
    let request = Request::new(NEXT_ID.fetch_add(1, Ordering::Relaxed), operation);
    let mut bytes = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err("plugin request exceeds message limit".into());
    }
    let child = Command::new(&plugin.executable)
        .args(&plugin.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("{}: {e}", plugin.name))?;
    let group = child.id() as i32;
    let mut process = RequestProcess { child, group };
    let mut input = Some(process.child.stdin.take().expect("piped stdin"));
    let mut output = process.child.stdout.take().expect("piped stdout");
    let mut errors = process.child.stderr.take().expect("piped stderr");
    nonblocking(input.as_ref().expect("piped stdin"))?;
    nonblocking(&output)?;
    nonblocking(&errors)?;
    let mut sent = 0;
    let mut data = Vec::new();
    let mut diagnostics = Vec::new();
    let mut status = None;
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            return Err("plugin request timed out".into());
        }
        if let Some(pipe) = &mut input {
            match pipe.write(&bytes[sent..]) {
                Ok(0) => return Err("plugin closed its request stream".into()),
                Ok(n) => sent += n,
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(e.to_string()),
            }
            if sent == bytes.len() {
                input = None;
            }
        }
        let output_eof = read_step(&mut output, &mut data, MAX_MESSAGE_BYTES)?;
        let error_eof = read_step(&mut errors, &mut diagnostics, 4096)?;
        if status.is_none() {
            status = process.child.try_wait().map_err(|e| e.to_string())?;
        }
        if let Some(status) = status {
            if !status.success() {
                return Err(format!("{} exited with {status}", plugin.name));
            }
            if output_eof && error_eof {
                break;
            }
        }
        // Nonblocking reads keep the deadline effective even if a descendant retains a pipe.
        thread::sleep(Duration::from_millis(1));
    }
    let response: Response = serde_json::from_slice(&data)
        .map_err(|_| "plugin returned invalid JSON response".to_string())?;
    validate_reply(&request, response)
}

fn validate_reply(request: &Request, response: Response) -> Result<Reply, String> {
    if response.version != VERSION {
        return Err("plugin protocol version mismatch".into());
    }
    if response.id != request.id {
        return Err("plugin response ID mismatch".into());
    }
    let reply = match response.outcome {
        Outcome::Ok(r) => r,
        Outcome::Error(e) => return Err(e.to_string()),
    };
    let valid = match (&request.operation, &reply) {
        (Operation::Describe, Reply::Description { .. })
        | (Operation::Diagnose, Reply::Diagnosis { .. })
        | (Operation::Activate { .. }, Reply::Activation(_)) => true,
        (Operation::Resolve { process }, Reply::Resolution(r)) => process == &r.process,
        _ => false,
    };
    if !valid {
        return Err("plugin returned a reply for a different method or process".into());
    }
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(code: &str) -> PluginConfig {
        PluginConfig {
            name: "fixture".into(),
            protocol_version: VERSION,
            executable: "/usr/bin/python3".into(),
            args: vec!["-c".into(), code.into()],
            match_executables: vec!["fixture".into()],
        }
    }
    #[test]
    fn subprocess_exchange_crash_malformed_timeout_and_recovery() {
        let good = fixture(
            "import sys,json; r=json.load(sys.stdin); print(json.dumps({'version':1,'id':r['id'],'status':'ok','data':{'kind':'diagnosis','value':{'available':True,'message':'ready'}}}))",
        );
        assert!(matches!(
            call(&good, Operation::Diagnose, Duration::from_secs(2)).unwrap(),
            Reply::Diagnosis {
                available: true,
                ..
            }
        ));
        for code in [
            "raise SystemExit(2)",
            "print('bad')",
            "import time; time.sleep(30)",
            "import sys; sys.stdout.write('x'*1100000)",
            "import sys; sys.stderr.write('x'*1100000)",
        ] {
            assert!(
                call(
                    &fixture(code),
                    Operation::Diagnose,
                    Duration::from_millis(150)
                )
                .is_err(),
                "{code}"
            );
        }
        assert!(call(&good, Operation::Diagnose, Duration::from_secs(2)).is_ok());
    }

    #[test]
    fn inherited_pipes_cannot_extend_the_request_deadline() {
        let plugin = fixture(
            "import subprocess; subprocess.Popen(['/usr/bin/sleep','2'], start_new_session=True)",
        );
        let start = Instant::now();
        assert!(call(&plugin, Operation::Describe, Duration::from_millis(100)).is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn response_must_match_version_id_method_and_identity() {
        let req = Request::new(5, Operation::Diagnose);
        let make = |version, id, reply| Response {
            version,
            id,
            outcome: Outcome::Ok(reply),
        };
        let reply = || Reply::Diagnosis {
            available: true,
            message: String::new(),
        };
        assert!(validate_reply(&req, make(2, 5, reply())).is_err());
        assert!(validate_reply(&req, make(1, 9, reply())).is_err());
        assert!(
            validate_reply(&Request::new(5, Operation::Describe), make(1, 5, reply())).is_err()
        );
        assert!(validate_reply(&req, make(1, 5, reply())).is_ok());
    }

    #[test]
    fn resolved_identity_must_match_the_requested_lifetime() {
        use top_dog_protocol::{ProcessIdentity, Resolution};
        let process = ProcessIdentity {
            pid: 42,
            start_ticks: 100,
        };
        let request = Request::new(1, Operation::Resolve { process });
        let response = Response {
            version: VERSION,
            id: 1,
            outcome: Outcome::Ok(Reply::Resolution(Resolution {
                process: ProcessIdentity {
                    pid: 42,
                    start_ticks: 101,
                },
                application: "fixture".into(),
                summary: String::new(),
                targets: vec![],
            })),
        };
        assert!(validate_reply(&request, response).is_err());
    }
}
