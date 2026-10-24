# Validation status

Validated in the implementation workspace on 2026-10-24 with Rust 1.93.0 on Linux.

| Check | Result |
| --- | --- |
| Workspace Rust tests | 40 passed; one live Firefox test intentionally ignored |
| Firefox bridge fixtures | Six passed |
| Pseudo-terminal tests | Five passed |
| Firefox executable protocol harness | Passed |
| Rust plugin template protocol harness | Passed |
| Formatting and Clippy with warnings denied | Passed |
| Release workspace build | Passed |
| Generated schema consistency | Passed |

The tests cover controlled metric calculations, parsing, identity changes, process filtering and selection, plugin protocol errors, crashes, output limits, timeouts, retained output pipes, browser mappings, private-tab exclusion, and stale targets. Disposable Linux tasks verify signal delivery and renice. Terminal tests exercise single-target activation and shared-target selection with fixture plugins, plus keyboard filtering, confirmation cancellation, resizing, and restoration after normal quit, Ctrl-C, and SIGTERM.

Live Firefox mapping, packaged-browser compatibility, and GNOME Wayland foreground focus remain unverified in this environment. Its sandbox denies loopback socket creation with `EPERM`, so it cannot connect to Firefox's automation listener. The installed Firefox executable reports version 157.0.1, but running `--version` does not establish integration compatibility.

The ignored Rust integration test creates a disposable profile and checks live PID mapping, existing-tab selection without increasing tab count, and rejection after closure. CI includes a separate job that installs Firefox and runs this test. That job has been configured here; it has not been executed by a hosted runner during this implementation.

Before treating the Firefox integration as desktop-verified, run the live test outside the sandbox, then check `--doctor`, shared-tab navigation, and actual window focus on Ubuntu GNOME Wayland. Snap and Flatpak PID namespace translation is outside this release.
