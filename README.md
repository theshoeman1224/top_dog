# Top Dog

A Rust terminal process monitor for Linux. Select a PID, inspect its details, and use an application plugin to navigate to the work behind it.

The first plugin maps Firefox content PIDs to existing tabs and embedded frames. Shared processes produce a tab chooser. CPU and memory remain process measurements; a shared PID cannot establish which tab caused the usage.

## Build and run

Requires Linux and Rust 1.88 or newer. Tests also use Python 3; the Firefox bridge fixtures use Node.js 20 or newer.

```bash
cargo build --release --workspace --locked
./target/release/top-dog
```

Or install the two application binaries:

```bash
cargo install --path crates/top-dog --locked
cargo install --path plugins/firefox --locked
```

The monitor works without plugins. Start with [Firefox setup](docs/firefox.md) to enable tab navigation. Configuration defaults to `$XDG_CONFIG_HOME/top-dog/config.toml`, or `~/.config/top-dog/config.toml`. [examples/config.toml](examples/config.toml) runs directly from this checkout after a release build:

```bash
./target/release/top-dog --config examples/config.toml --doctor
./target/release/top-dog --config examples/config.toml
```

## Controls

| Key | Action |
| --- | --- |
| Up/Down, j/k | Select a process |
| PageUp/PageDown, Home/End | Scroll |
| P / M / N / T / C | Sort by CPU / RAM / PID / CPU time / command |
| r | Reverse sort |
| / / p / u | Filter by text / PID / user or UID |
| Esc | Clear filters or close a dialog |
| Enter | Inspect process and list application targets |
| o | Open the single matching target, or choose among several |
| Space | Pause the displayed snapshot |
| d | Set refresh interval in milliseconds |
| s / n | Signal / renice, followed by explicit confirmation |
| ? / q / Ctrl-C | Help / quit / quit |

CPU uses 100% per logical CPU and can exceed 100%. The first sample shows `--` because CPU requires two samples. RAM percentage uses process RSS divided by physical memory. Shared pages can appear in multiple processes' RSS.

```bash
top-dog --sort memory --filter firefox
top-dog --json --iterations 2 --refresh-ms 500
top-dog --batch --user "$USER"
top-dog --help
```

The process table truncates long usernames with `…` and marks numbers that exceed their column width with `#`.

Batch and JSON modes apply the same sorting and filters. JSON emits one snapshot per line.

## Plugins and desktop support

Plugins are explicitly registered executable programs using a versioned JSON Lines interface. They run as your user and need no root privileges. Only configure executables you trust; subprocess isolation limits hangs and protocol failures, and does not sandbox plugin code.

- [Plugin interface and authoring](docs/plugins.md)
- [Working Rust template](templates/rust-plugin)
- [AI plugin authoring prompt](docs/plugin-ai-prompt.md)
- [Architecture and desktop extension points](docs/architecture.md)

Ubuntu GNOME Wayland is the initial target. Firefox selects its existing tab and requests native window focus. The desktop controls foreground activation, which the application reports separately. Additional desktop implementations belong behind `DesktopActivation`.

This release implements core monitoring and process controls. It does not claim full procps `top` option, display, thread-view, or configuration compatibility. Ghostty and other application plugins are follow-up work. Firefox internals can change between releases; run `--doctor` to check connectivity and system access.

## Tests

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
node plugins/firefox/tests/bridge.test.cjs
cargo build --workspace --locked
python3 scripts/check-plugin.py target/debug/top-dog-plugin-template
python3 scripts/check-plugin.py target/debug/top-dog-firefox
python3 scripts/test-terminal.py target/debug/top-dog
```

The live Firefox test is opt-in and uses a newly created disposable profile:

```bash
TOP_DOG_FIREFOX_BINARY=/path/to/firefox \
  cargo test -p top-dog-firefox --test live --locked -- --ignored
```

It requires permission to create loopback sockets. A headless test verifies mapping and existing-tab selection; foreground focus still requires a desktop check. See [validation status](docs/validation.md) for checks performed in the implementation environment.
