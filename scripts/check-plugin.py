#!/usr/bin/env python3
"""Check a plugin executable's protocol handling without activating targets."""
import argparse
import json
import pathlib
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--resolve-pid", type=int)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if not args.command:
        parser.error("provide the plugin executable followed by its arguments")
    requests = [
        {"version": 1, "id": 1, "method": "describe"},
        {"version": 1, "id": 2, "method": "diagnose"},
        {"version": 99, "id": 3, "method": "describe"},
        {"version": 1, "id": 4, "method": "describe"},
    ]
    if args.resolve_pid is not None:
        if args.resolve_pid <= 0:
            parser.error("PID must be positive")
        stat = pathlib.Path(f"/proc/{args.resolve_pid}/stat").read_text()
        process = {"pid": args.resolve_pid, "start_ticks": int(stat.rsplit(")", 1)[1].split()[19])}
        requests.append({"version": 1, "id": 5, "method": "resolve", "params": {"process": process}})
    payload = "malformed\n" + "".join(json.dumps(r) + "\n" for r in requests)
    output = subprocess.run(args.command, input=payload, text=True, capture_output=True, timeout=30, check=True)
    if len(output.stdout.encode()) > 6 * 1024 * 1024:
        raise ValueError("responses exceed contract-test output limit")
    replies = [json.loads(line) for line in output.stdout.splitlines()]
    assert len(replies) == len(requests) + 1, "expected exactly one response per request"
    first = replies.pop(0)
    assert first["id"] == 0 and first["status"] == "error"
    assert first["data"]["code"] == "invalid_request"
    for request, reply in zip(requests, replies):
        assert reply["version"] == 1 and reply["id"] == request["id"]
        if request["version"] != 1:
            assert reply["status"] == "error" and reply["data"]["code"] == "incompatible_version"
            continue
        if request["method"] == "resolve" and reply["status"] == "error":
            assert isinstance(reply["data"]["message"], str) and isinstance(reply["data"]["code"], str)
            continue
        assert reply["status"] == "ok", f"{request['method']} returned an error"
        data = reply["data"]
        expected = {"describe": "description", "diagnose": "diagnosis", "resolve": "resolution"}[request["method"]]
        assert data["kind"] == expected
        value = data["value"]
        if expected == "description":
            assert isinstance(value["name"], str) and isinstance(value["version"], str)
            assert isinstance(value["capabilities"], list)
        elif expected == "diagnosis":
            assert isinstance(value["available"], bool) and isinstance(value["message"], str)
        else:
            assert value["process"] == request["params"]["process"]
            assert isinstance(value["targets"], list) and isinstance(value["summary"], str)
            for target in value["targets"]:
                assert isinstance(target["id"], str) and isinstance(target["title"], str)
                assert isinstance(target["can_activate"], bool)
                assert target["relationship"] in {"main_page", "embedded_frame", "application"}
    print("Plugin contract passed. Application availability and live activation require separate checks.")


if __name__ == "__main__":
    try:
        main()
    except (AssertionError, OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"Plugin contract failed: {error}", file=sys.stderr)
        sys.exit(1)
