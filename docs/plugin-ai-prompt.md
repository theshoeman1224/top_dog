# AI plugin authoring prompt

Copy the prompt below into an agent working in this repository. Fill in the application, version, desktop, and required navigation target.

```text
Implement a Top Dog executable plugin for:
Application: <name>
Application versions: <versions>
Linux desktop/session: <desktop, X11 or Wayland>
Target to inspect and activate: <existing tab, terminal pane, document, window, etc.>
Allowed application setup: <extension, explicit remote-control setting, helper, or none>

Read docs/plugins.md and the shared Rust protocol types first. Start from
templates/rust-plugin and register a uniquely named package in this workspace.
Use plugins/firefox as a reference for process ownership and stale-target checks.

1. Verify the application's supported inspection and control interfaces against
   primary documentation or source. Record links and tested versions. Establish
   how OS PIDs map to application targets, including shared processes and frames.
   If exact mapping or activation is unavailable, implement useful diagnosis and
   inspection and report the unsupported capability explicitly.

2. Implement describe, diagnose, resolve, and activate through protocol v1.
   Preserve request IDs, process identities, and structured errors. Each request
   runs in a fresh subprocess. Encode opaque target identities that can be
   revalidated after process exit, PID reuse, navigation, closure, and app restart.
   Activate the existing target rather than opening its URL or creating a new one.
   Report focus separately from target selection, using DesktopActivation where
   native application activation is available.

3. Keep application logic inside the plugin. Change the core only if this
   application demonstrates a necessary, general interface change. Keep optional
   desktop handling behind the existing desktop interface. Match executables
   narrowly, then verify ownership before acting. Use explicit connection setup
   and keep credentials and sensitive target metadata out of diagnostics.

4. Supply a working plugin manifest, user setup instructions, capability limits,
   and tests for discovery, shared mappings, unavailable setup, stale identities,
   malformed responses, and activation outcomes. Add an opt-in integration test
   using a disposable application session where possible.

5. Run formatting checks, Clippy, workspace tests, and the plugin contract harness.
   Report exactly which live scenarios were verified and which remain untested.
   Completion requires runnable code, registration, documentation, and passing
   applicable checks. An unimplemented activation stub is acceptable only when
   the researched application cannot support activation, and that limitation is
   visible in diagnosis and documentation.
```
