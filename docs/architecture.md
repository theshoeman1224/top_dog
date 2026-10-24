# Architecture

The workspace contains three main modules and two plugin executables.

`top-dog-core` owns Linux sampling, identity checks, process controls, configuration, plugin execution, and the sortable process view. It reads `/proc` directly. Parsers and delta calculations accept controlled inputs for regression tests. Process identity is PID plus kernel start time; samples do not join metrics across PID reuse.

`top-dog` owns input state, rendering, CLI output, and terminal lifetime. A collector thread sends bounded snapshots to the UI. Plugin inspection and actions run on worker threads. Confirmation captures the original process identity even if sorting or collection changes the selected row. Crossterm handles terminal input and restoration; the small renderer produces testable lines and clips by Unicode display width.

`top-dog-protocol` owns the versioned wire types, schemas, and bounded JSON Lines server. The host invokes explicit executable paths and limits deadlines and message sizes. Each request gets its own process group. This protects the monitor from plugin protocol failures and hangs; it is not a security sandbox.

`top-dog-firefox` owns Marionette framing, explicit endpoints, browser ownership, tab/frame mapping, and target activation. It sends a fixed bridge script into a fresh privileged browser sandbox. It never evaluates page JavaScript. Firefox target IDs include the browser lifetime and the current matched browsing-context/document identities.

`top-dog-plugin-template` demonstrates the same executable protocol. It supplies inspection structure while reporting unsupported activation honestly.

## Adding desktops

`DesktopActivation` separates a native application's focus request from target selection. `NativeDesktop` asks the application to focus its selected window and returns `Requested` or `Denied`; it does not claim compositor confirmation. The Firefox plugin supplies the actual browser focus request as a callback.

To add a desktop-specific activation path, implement that interface, select the implementation from desktop/session information, and keep compositor APIs inside it. Preserve the application-native fallback. Add controlled tests for requested, denied, unsupported, and verified focus outcomes. PID discovery and plugin protocol types do not need to change.

## Linux limitations

- Signal delivery uses a pidfd to keep PID reuse from redirecting a signal. This requires a kernel supporting pidfds.
- Renice validates lifetime immediately before `setpriority`. Linux does not provide a pidfd equivalent of `setpriority`, so this operation has a narrow remaining PID-reuse race.
- Permissions can hide process details. The monitor skips unreadable processes and reports restricted entries without elevating privileges.
- Cross-namespace PID translation and remote-host monitoring are outside the first release.
- Refresh interval changes and UI preferences are session-local; configuration is read at startup.

Ratatui and the procfs crate were unavailable in the implementation sandbox's offline dependency cache. The terminal renderer and proc readers are contained in separate modules so these implementation choices can be changed without affecting plugins.
