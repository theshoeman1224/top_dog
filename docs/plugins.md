# Executable plugin interface

The authoritative Rust types are in `top-dog-protocol`. Generated schemas are in `schema/request.json` and `schema/response.json`.

## Lifecycle and framing

The host starts a fresh executable for each request, writes one UTF-8 JSON line, closes stdin, and reads one response. The shared `serve` function also supports several requests in one session for contract testing. Every request must work independently of previous requests, including activation.

Each message is at most 1 MiB, including the newline. stdout contains protocol messages only. stderr is reserved for diagnostics; the host bounds and discards it to avoid exposing URLs through errors. The host rejects extra responses, mismatched IDs, wrong reply kinds, and process mismatches. It terminates the request's process group after completion or timeout.

The default request deadline is five seconds, configurable from 100 through 30000 milliseconds. Plugin work must respect that deadline. The core monitor continues sampling and rendering while the plugin runs.

## Requests and responses

Every envelope includes `version: 1` and an integer `id`. A process identity contains a positive Linux `pid` and `start_ticks`, field 22 of `/proc/PID/stat` in kernel clock ticks.

```json
{"version":1,"id":1,"method":"resolve","params":{"process":{"pid":1234,"start_ticks":56789}}}
```

Successful responses have `status: "ok"` and tagged reply data:

```json
{"version":1,"id":1,"status":"ok","data":{"kind":"resolution","value":{"process":{"pid":1234,"start_ticks":56789},"application":"Example","summary":"One related window","targets":[{"id":"opaque-target-token","title":"Example window","url":null,"window":"7","relationship":"application","can_activate":true}]}}}
```

Errors have `status: "error"`:

```json
{"version":1,"id":1,"status":"error","data":{"code":"stale_process","message":"Process exited or its PID was reused"}}
```

| Method | Parameters | Reply kind |
| --- | --- | --- |
| `describe` | None | `description`, with name, version, capabilities |
| `diagnose` | None | `diagnosis`, with available and message |
| `resolve` | process | `resolution`, with application details and targets |
| `activate` | process, target_id | `activation`, with selected, focus, message |

Use `diagnosis.available: false` for missing application setup. Use structured errors for request failures. A malformed envelope gets id 0. An unsupported protocol version receives an `incompatible_version` error with the original request ID.

Targets use `main_page`, `embedded_frame`, or `application` relationships. Opaque identifiers must encode enough information to revalidate the original target in another plugin process. Recheck both process lifetime and target ownership before acting. An old PID, title, URL, or window number alone is insufficient to establish identity.

Activation reports target selection separately from focus. `requested` means the application requested focus; `confirmed` requires evidence that foreground focus succeeded. `denied` and `unsupported` express other outcomes. A successful IPC call alone does not justify `confirmed`.

## Registration

Add a `[[plugins]]` entry to the XDG configuration file. Use a unique name, `protocol_version = 1`, an executable path, optional arguments, and `match_executables`. Relative executable paths are resolved against the configuration file's directory. Arguments are passed directly, without shell evaluation or tilde/environment expansion.

Matching compares exact process names and executable basenames read from `/proc/PID/exe`, falling back to command lines when needed. It only selects candidate plugins; plugins must establish actual ownership. Matching plugins are tried in configuration order until one resolves successfully.

## Create a Rust plugin

1. Copy `templates/rust-plugin` to `plugins/<application>` inside this checkout.
2. Give its Cargo package a unique name and add its directory to the root workspace's members. The inherited workspace dependencies will then resolve automatically.
3. Implement diagnosis and target resolution using the application's documented interfaces. Implement activation only where you can identify an existing target reliably.
4. Add focused regression tests and a manifest based on `templates/rust-plugin/plugin.toml`.
5. Build the plugin and run `scripts/check-plugin.py` against its executable. Use `--resolve-pid PID` before the executable path to exercise discovery on a process you own.

For a standalone repository, replace the template's workspace inheritance with concrete package fields and path dependencies pointing to this checkout's `crates/core` and `crates/protocol`. The core dependency is optional if you implement your own Linux lifetime checks. Non-Rust implementations can use the generated schemas.

The template resolves to an empty target list and rejects activation. Its passing contract test proves protocol handling, not application integration. Use [the AI prompt](plugin-ai-prompt.md) to specify a complete application implementation.

Regenerate the schemas after changing shared types:

```bash
cargo run -p top-dog-protocol --example schema > schema/request.json
cargo run -p top-dog-protocol --example response-schema > schema/response.json
```
