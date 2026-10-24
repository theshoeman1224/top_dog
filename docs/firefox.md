# Firefox setup

Firefox's ordinary tabs extension interface does not expose the process mapping needed here. This plugin uses Marionette with privileged browser access, then walks each tab's browsing-context tree to find its OS PIDs. See Mozilla's [Marionette introduction](https://firefox-source-docs.mozilla.org/remote/marionette/Intro.html), [system-access flags](https://firefox-source-docs.mozilla.org/testing/geckodriver/Flags.html), and [task-manager source](https://github.com/mozilla-firefox/firefox/blob/main/toolkit/components/aboutprocesses/content/aboutProcesses.js).

## Enable the connection

1. Save your work and close Firefox normally. The required startup flags apply only to a newly started browser process.
2. Start your chosen profile with:

   ```bash
   firefox --marionette --remote-allow-system-access
   ```

3. Marionette defaults to port 2828. For a different profile or port, set `marionette.port` in that profile's `about:config`, restart with the flags, and configure the corresponding endpoint. Keep `marionette.force-local` enabled.
4. Register the plugin using [the example configuration](../examples/config.toml). If you installed it with Cargo, set `executable` to the absolute path of your installed `top-dog-firefox` binary. Run:

   ```bash
   top-dog --doctor
   top-dog --sort memory
   ```

5. Select a Firefox process and press `o`. A single matching tab is selected directly. Several matches open a chooser. Press Enter on a tab to select it.

System access gives connected automation clients broad access to Firefox and the host system. Marionette does not authenticate clients. Keep its listener on loopback and enable it only for a profile you intend to control. The plugin accepts numeric loopback addresses only and never changes your preferences or restarts Firefox for you. Mozilla describes this access in its [remote-agent security documentation](https://firefox-source-docs.mozilla.org/remote/Security.html).

To disable the integration, restart Firefox without the automation flags.

## Multiple profiles

Give each simultaneously running profile a different Marionette port. Configure the plugin with repeated endpoint arguments:

```toml
args = ["--endpoint", "127.0.0.1:2828", "--endpoint", "127.0.0.1:2829"]
```

The plugin verifies that the selected process descends from the browser instance at that endpoint. Increase `plugin_timeout_ms` for several endpoints if necessary, up to 30000 milliseconds. `--doctor` reports each endpoint's status; process resolution uses the first configured endpoint that owns the process.

## What the result means

- A main-page match identifies a tab whose top-level page runs in the selected process.
- An embedded-frame match identifies a tab containing a frame running in that process. It does not prove that the top-level page is responsible for all resource usage.
- Several tabs may share a process. The chooser lists related tabs without inventing individual CPU or memory measurements.
- Browser, network, GPU, extension, worker, and other helper processes may have no navigable tab. The monitor still shows their Linux details.
- Discarded tabs without live content have no content PID to match.
- Tab activation selects the existing tab. It never launches its URL in a new tab. Closed, navigated, or migrated targets require a fresh inspection.
- Wayland and other desktops may deny foreground focus even after tab selection succeeds. Switch to Firefox if its window stays behind the terminal.

Private windows are excluded by default. Add `--include-private` to the plugin arguments only if you want private tab titles and URLs in the monitor. Tab metadata stays in memory and is not logged by the bundled plugin or host. Process command lines, as with `top`, are visible in the monitor and batch output.

## Connection problems

Run `top-dog --doctor`. Check that the endpoint, profile, startup flags, and port agree. Another automation client may already own the browser session. Browser updates can also change privileged interfaces.

Marionette itself may focus a browser window when establishing a session, including for diagnosis or inspection. The plugin creates a session for each request and detaches afterward. Browser-owned connection behavior is separate from the plugin's explicit target selection and focus request.

Snap, Flatpak, containers, or restricted `/proc` mounts may prevent ancestry inspection or present a different PID namespace. The plugin reports unsupported mapping rather than guessing. The initial release does not translate sandbox PIDs or bypass package confinement. An unpacked Mozilla Firefox build can be used for the disposable-profile test.

The headless integration test creates and removes its own profile and never attaches to your current session. Set `TOP_DOG_FIREFOX_BINARY` to a runnable Firefox executable and use the command in the README.
