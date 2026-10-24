use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread,
};
use top_dog_core::{
    config::{Config, PluginConfig},
    monitor, plugin,
    view::{Sort, View},
};
use top_dog_protocol::{Operation, ProcessIdentity, Reply, Resolution};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Search,
    Pid,
    User,
    Refresh,
    Signal,
    Nice,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Signal(i32),
    Nice(i32),
}

pub enum Mode {
    Normal,
    Input(Input, String),
    Confirm {
        process: ProcessIdentity,
        command: String,
        action: Action,
    },
    Targets {
        plugin: PluginConfig,
        resolution: Resolution,
        cursor: usize,
    },
    Help,
}

enum Completed {
    Inspect {
        process: ProcessIdentity,
        details: Result<String, String>,
        resolution: Box<Result<Option<(PluginConfig, Resolution)>, String>>,
        open: bool,
    },
    Status(Result<String, String>),
}

pub struct App {
    pub view: View,
    pub config: Config,
    pub mode: Mode,
    pub status: String,
    pub details: Vec<String>,
    pub busy: bool,
    pub quit: bool,
    pub refresh_ms: u64,
    tx: Sender<Completed>,
    rx: Receiver<Completed>,
}

impl App {
    pub fn new(config: Config) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            refresh_ms: config.refresh_ms,
            config,
            view: View::default(),
            mode: Mode::Normal,
            status: "Enter: inspect   o: open application target   ?: help".into(),
            details: vec![],
            busy: false,
            quit: false,
            tx,
            rx,
        }
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(result) = self.rx.try_recv() {
            changed = true;
            self.busy = false;
            match result {
                Completed::Status(result) => {
                    self.status = result.unwrap_or_else(|e| format!("Error: {e}"))
                }
                Completed::Inspect {
                    process,
                    details,
                    resolution,
                    open,
                } => {
                    if self.view.selected != Some(process) {
                        continue;
                    }
                    self.details = details
                        .unwrap_or_else(|e| format!("Details unavailable: {e}"))
                        .lines()
                        .map(str::to_string)
                        .collect();
                    match *resolution {
                        Ok(Some((plugin, resolution))) if !resolution.targets.is_empty() => {
                            self.status = resolution.summary.clone();
                            if !matches!(self.mode, Mode::Normal) {
                                self.status.push_str(
                                    " Press Enter again after closing the current dialog.",
                                );
                            } else if open
                                && resolution.targets.len() == 1
                                && resolution.targets[0].can_activate
                            {
                                self.activate(
                                    plugin,
                                    resolution.process,
                                    resolution.targets[0].id.clone(),
                                );
                            } else {
                                self.mode = Mode::Targets {
                                    plugin,
                                    resolution,
                                    cursor: 0,
                                };
                            }
                        }
                        Ok(Some((_, resolution))) => self.status = resolution.summary,
                        Ok(None) => {
                            let firefox = self.view.selected_process().is_some_and(|process| {
                                let executable = process
                                    .executable
                                    .as_deref()
                                    .or_else(|| process.command.split_whitespace().next())
                                    .and_then(|path| std::path::Path::new(path).file_name())
                                    .and_then(|name| name.to_str());
                                matches!(process.name.as_str(), "firefox" | "firefox-bin")
                                    || matches!(executable, Some("firefox" | "firefox-bin"))
                            });
                            self.status = if firefox {
                                "No Firefox plugin configured for this application.\n\
                                 Install top-dog-firefox if needed.\n\
                                 Register top-dog-firefox in your top-dog config.\n\
                                 Close Firefox normally, then restart with:\n\
                                 firefox --marionette --remote-allow-system-access\n\
                                 Run top-dog --doctor. Setup: docs/firefox.md"
                            } else {
                                "No plugin configured for this application. Process details are available below."
                            }.into();
                        }
                        Err(e) => self.status = format!("Integration unavailable: {e}"),
                    }
                }
            }
        }
        changed
    }

    pub fn update_snapshot(&mut self, snapshot: monitor::Snapshot) {
        let previous = self.view.selected;
        self.view.replace(snapshot);
        if self.view.selected != previous {
            self.details.clear();
        }
    }

    pub fn key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        let mode = std::mem::replace(&mut self.mode, Mode::Normal);
        match mode {
            Mode::Input(kind, mut text) => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => {
                    if let Err(e) = self.submit(kind, &text) {
                        self.status = e;
                        self.mode = Mode::Input(kind, text);
                    }
                }
                KeyCode::Backspace => {
                    text.pop();
                    self.mode = Mode::Input(kind, text);
                }
                KeyCode::Char(c) if !c.is_control() && text.len() < 512 => {
                    text.push(c);
                    self.mode = Mode::Input(kind, text);
                }
                _ => self.mode = Mode::Input(kind, text),
            },
            Mode::Confirm {
                process,
                command,
                action,
            } => match key.code {
                KeyCode::Char('y') => self.perform(process, action),
                KeyCode::Char('n') | KeyCode::Esc => self.status = "Action cancelled".into(),
                _ => {
                    self.mode = Mode::Confirm {
                        process,
                        command,
                        action,
                    }
                }
            },
            Mode::Help => {
                if !matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')
                ) {
                    self.mode = Mode::Help;
                }
            }
            Mode::Targets {
                plugin,
                resolution,
                mut cursor,
            } => {
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => cursor = cursor.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => {
                        cursor = (cursor + 1).min(resolution.targets.len().saturating_sub(1))
                    }
                    KeyCode::Enter | KeyCode::Char('o') => {
                        if let Some(target) = resolution.targets.get(cursor) {
                            if target.can_activate {
                                self.activate(plugin, resolution.process, target.id.clone());
                                return;
                            }
                            self.status = "This target cannot be activated".into();
                        }
                    }
                    KeyCode::Esc | KeyCode::Char('q') => return,
                    _ => {}
                }
                self.mode = Mode::Targets {
                    plugin,
                    resolution,
                    cursor,
                };
            }
            Mode::Normal => self.normal_key(key.code),
        }
    }

    fn normal_key(&mut self, code: KeyCode) {
        let before = self.view.selected;
        match code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Up | KeyCode::Char('k') => self.view.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') => self.view.move_selection(1),
            KeyCode::PageUp => self.view.move_selection(-15),
            KeyCode::PageDown => self.view.move_selection(15),
            KeyCode::Home => self.view.move_selection(isize::MIN),
            KeyCode::End => self.view.move_selection(isize::MAX),
            KeyCode::Char('P') => self.view.sort = Sort::Cpu,
            KeyCode::Char('M') => self.view.sort = Sort::Memory,
            KeyCode::Char('N') => self.view.sort = Sort::Pid,
            KeyCode::Char('T') => self.view.sort = Sort::Time,
            KeyCode::Char('C') => self.view.sort = Sort::Command,
            KeyCode::Char('r') => self.view.reverse = !self.view.reverse,
            KeyCode::Char(' ') => self.view.paused = !self.view.paused,
            KeyCode::Char('/') => self.mode = Mode::Input(Input::Search, self.view.text.clone()),
            KeyCode::Char('p') => {
                self.mode = Mode::Input(
                    Input::Pid,
                    self.view.pid.map(|p| p.to_string()).unwrap_or_default(),
                )
            }
            KeyCode::Char('u') => self.mode = Mode::Input(Input::User, self.view.user.clone()),
            KeyCode::Char('d') => {
                self.mode = Mode::Input(Input::Refresh, self.refresh_ms.to_string())
            }
            KeyCode::Char('s') if self.view.selected.is_some() => {
                self.mode = Mode::Input(Input::Signal, "TERM".into())
            }
            KeyCode::Char('n') if self.view.selected.is_some() => {
                self.mode = Mode::Input(Input::Nice, "0".into())
            }
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Esc => {
                self.view.text.clear();
                self.view.pid = None;
                self.view.user.clear();
            }
            KeyCode::Enter => self.inspect(false),
            KeyCode::Char('o') => self.inspect(true),
            _ => {}
        }
        self.view.reconcile();
        if self.view.selected != before {
            self.details.clear();
        }
    }

    fn submit(&mut self, kind: Input, text: &str) -> Result<(), String> {
        let text = text.trim();
        match kind {
            Input::Search => self.view.text = text.into(),
            Input::Pid => {
                self.view.pid = if text.is_empty() {
                    None
                } else {
                    Some(
                        text.parse::<i32>()
                            .ok()
                            .filter(|p| *p > 0)
                            .ok_or("PID must be a positive integer or empty")?,
                    )
                }
            }
            Input::User => self.view.user = text.into(),
            Input::Refresh => {
                let n = text
                    .parse::<u64>()
                    .map_err(|_| "Enter refresh milliseconds, 100 to 60000")?;
                if !(100..=60_000).contains(&n) {
                    return Err("Refresh must be 100 to 60000 milliseconds".into());
                }
                self.refresh_ms = n;
            }
            Input::Signal | Input::Nice => {
                let p = self
                    .view
                    .selected_process()
                    .ok_or("Select a process first")?;
                let action = if kind == Input::Nice {
                    let n = text
                        .parse::<i32>()
                        .map_err(|_| "Nice value must be -20 to 19")?;
                    if !(-20..=19).contains(&n) {
                        return Err("Nice value must be -20 to 19".into());
                    }
                    Action::Nice(n)
                } else {
                    Action::Signal(parse_signal(text)?)
                };
                self.mode = Mode::Confirm {
                    process: p.id(),
                    command: p.command.clone(),
                    action,
                };
            }
        }
        self.view.reconcile();
        Ok(())
    }

    fn inspect(&mut self, open: bool) {
        if self.busy {
            self.status = "An operation is already running".into();
            return;
        }
        let Some(process) = self.view.selected_process().cloned() else {
            return;
        };
        let matches: Vec<_> = self
            .config
            .plugins
            .iter()
            .filter(|p| p.matches(&process))
            .cloned()
            .collect();
        let timeout = self.config.timeout();
        let tx = self.tx.clone();
        self.busy = true;
        self.status = "Inspecting selected process...".into();
        thread::spawn(move || {
            let id = process.id();
            let details = monitor::details(id);
            let mut resolution = Ok(None);
            for plugin_config in matches {
                match plugin::call(&plugin_config, Operation::Resolve { process: id }, timeout) {
                    Ok(Reply::Resolution(r)) => {
                        resolution = Ok(Some((plugin_config, r)));
                        break;
                    }
                    Ok(_) => resolution = Err("wrong plugin reply".into()),
                    Err(e) => resolution = Err(e),
                }
            }
            let _ = tx.send(Completed::Inspect {
                process: id,
                details,
                resolution: Box::new(resolution),
                open,
            });
        });
    }

    fn activate(&mut self, config: PluginConfig, process: ProcessIdentity, target_id: String) {
        if self.busy {
            self.status = "An operation is already running".into();
            return;
        }
        let tx = self.tx.clone();
        let timeout = self.config.timeout();
        self.busy = true;
        self.status = "Selecting application target...".into();
        thread::spawn(move || {
            let result = monitor::validate(process)
                .and_then(|_| {
                    plugin::call(&config, Operation::Activate { process, target_id }, timeout)
                })
                .and_then(|reply| match reply {
                    Reply::Activation(a) if a.selected => Ok(a.message),
                    Reply::Activation(a) => {
                        Err(format!("Target selection was not confirmed: {}", a.message))
                    }
                    _ => Err("unexpected activation reply".into()),
                });
            let _ = tx.send(Completed::Status(result));
        });
    }

    fn perform(&mut self, process: ProcessIdentity, action: Action) {
        if self.busy {
            self.status = "An operation is already running".into();
            return;
        }
        let tx = self.tx.clone();
        self.busy = true;
        thread::spawn(move || {
            let result = match action {
                Action::Signal(signal) => monitor::signal(process, signal),
                Action::Nice(nice) => monitor::renice(process, nice),
            }
            .map(|_| format!("Applied {action:?} to PID {}", process.pid));
            let _ = tx.send(Completed::Status(result));
        });
    }
}

pub fn parse_signal(text: &str) -> Result<i32, String> {
    match text.to_uppercase().trim_start_matches("SIG") {
        "TERM" | "15" => Ok(libc::SIGTERM),
        "KILL" | "9" => Ok(libc::SIGKILL),
        "STOP" | "19" => Ok(libc::SIGSTOP),
        "CONT" | "18" => Ok(libc::SIGCONT),
        "HUP" | "1" => Ok(libc::SIGHUP),
        "INT" | "2" => Ok(libc::SIGINT),
        _ => Err("Use TERM, KILL, STOP, CONT, HUP, or INT".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use top_dog_core::monitor::{Process, Snapshot};
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    #[test]
    fn confirmation_captures_original_process_and_cancel_has_no_effect() {
        let mut app = App::new(Config::default());
        let id = ProcessIdentity {
            pid: 123,
            start_ticks: 50,
        };
        app.view.replace(Snapshot {
            processes: vec![Process {
                identity: Some(id),
                command: "test".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        app.key(key(KeyCode::Char('s')));
        app.key(key(KeyCode::Enter));
        assert!(
            matches!(app.mode,Mode::Confirm {process,action:Action::Signal(libc::SIGTERM),..} if process==id)
        );
        app.key(key(KeyCode::Char('n')));
        assert!(matches!(app.mode, Mode::Normal));
        assert!(!app.busy);
    }
    #[test]
    fn input_validation_pause_help_and_quit_transitions() {
        let mut app = App::new(Config::default());
        assert!(app.submit(Input::Refresh, "0").is_err());
        assert!(app.submit(Input::Nice, "20").is_err());
        assert!(app.submit(Input::Pid, "-1").is_err());
        app.submit(Input::Search, "firefox").unwrap();
        assert_eq!(app.view.text, "firefox");
        app.key(key(KeyCode::Esc));
        assert!(app.view.text.is_empty());
        app.key(key(KeyCode::Char(' ')));
        assert!(app.view.paused);
        app.key(key(KeyCode::Char('?')));
        assert!(matches!(app.mode, Mode::Help));
        app.key(key(KeyCode::Esc));
        app.key(key(KeyCode::Char('q')));
        assert!(app.quit);
    }
    #[test]
    fn signal_names_are_bounded() {
        assert_eq!(parse_signal("SIGTERM").unwrap(), libc::SIGTERM);
        assert!(parse_signal("0").is_err());
        assert!(parse_signal("999").is_err());
    }

    #[test]
    fn stale_inspection_cannot_open_a_target_for_a_different_selection() {
        let mut app = App::new(Config::default());
        app.busy = true;
        app.tx
            .send(Completed::Inspect {
                process: ProcessIdentity {
                    pid: 42,
                    start_ticks: 1,
                },
                details: Ok("stale details".into()),
                resolution: Box::new(Ok(None)),
                open: true,
            })
            .unwrap();
        app.poll();
        assert!(!app.busy);
        assert!(app.details.is_empty());
        assert!(matches!(app.mode, Mode::Normal));
    }

    #[test]
    fn unconfigured_firefox_shows_setup_in_the_rendered_screen() {
        for process in [
            Process {
                name: "Isolated Web Co".into(),
                executable: Some("/snap/firefox/9036/usr/lib/firefox/firefox".into()),
                ..Default::default()
            },
            Process {
                name: "Isolated Web Co".into(),
                command: "/usr/lib/firefox/firefox-bin -contentproc".into(),
                ..Default::default()
            },
            Process {
                name: "firefox".into(),
                ..Default::default()
            },
        ] {
            for open in [false, true] {
                let mut app = App::new(Config::default());
                let id = ProcessIdentity {
                    pid: 63591,
                    start_ticks: 474854,
                };
                app.view.replace(Snapshot {
                    processes: vec![Process {
                        identity: Some(id),
                        ..process.clone()
                    }],
                    ..Default::default()
                });
                app.tx.send(Completed::Inspect {
                    process: id,
                    details: Ok("Executable: Firefox\nWorking directory: /proc/63591/fdinfo\nStart ticks: 474854\nPID namespace: pid:[4026531836]".into()),
                    resolution: Box::new(Ok(None)),
                    open,
                }).unwrap();
                assert!(app.poll());
                for width in [60, 80, 120] {
                    let lines = crate::render::screen(&app, width, 24);
                    let screen = lines
                        .iter()
                        .map(|line| line.text.as_str())
                        .collect::<Vec<_>>()
                        .join("\n");
                    for expected in [
                        "No Firefox plugin configured",
                        "Register top-dog-firefox",
                        "top-dog config",
                        "Close Firefox normally",
                        "firefox --marionette --remote-allow-system-access",
                        "top-dog --doctor",
                        "docs/firefox.md",
                        "Start ticks: 474854",
                    ] {
                        assert!(
                            screen.contains(expected),
                            "missing {expected:?} at width {width}:\n{screen}"
                        );
                    }
                    assert!(lines.len() <= 24);
                }
            }
        }
    }

    #[test]
    fn unrelated_process_keeps_generic_missing_plugin_message() {
        let mut app = App::new(Config::default());
        let id = ProcessIdentity {
            pid: 42,
            start_ticks: 1,
        };
        app.view.replace(Snapshot {
            processes: vec![Process {
                identity: Some(id),
                name: "bash".into(),
                command: "bash firefox.sh".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        app.tx
            .send(Completed::Inspect {
                process: id,
                details: Ok("details".into()),
                resolution: Box::new(Ok(None)),
                open: false,
            })
            .unwrap();
        assert!(app.poll());
        assert!(
            app.status
                .starts_with("No plugin configured for this application.")
        );
        assert!(!app.status.contains("Firefox"));
    }

    #[test]
    fn replaced_selected_process_clears_its_details() {
        let mut app = App::new(Config::default());
        app.view.selected = Some(ProcessIdentity {
            pid: 42,
            start_ticks: 1,
        });
        app.details = vec!["old process details".into()];
        app.update_snapshot(Snapshot::default());
        assert!(app.details.is_empty());
    }
}
