use crate::app::{App, Mode};
use top_dog_core::{display_text, view::View};
use unicode_width::UnicodeWidthChar;

pub struct Line {
    pub text: String,
    pub selected: bool,
}
impl From<String> for Line {
    fn from(text: String) -> Self {
        Self {
            text,
            selected: false,
        }
    }
}

pub fn clip(text: &str, width: usize) -> String {
    let text = display_text(text);
    let mut n = 0;
    text.chars()
        .take_while(|c| {
            n += c.width().unwrap_or(0);
            n <= width
        })
        .collect()
}

pub fn bytes(n: u64) -> String {
    let units = ["B", "K", "M", "G", "T"];
    let mut number = n as f64;
    let mut unit = 0;
    while number >= 1024.0 && unit < units.len() - 1 {
        number /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n}B")
    } else {
        format!("{number:.1}{}", units[unit])
    }
}

fn percentage(n: Option<f64>) -> String {
    n.map(|v| format!("{v:.1}")).unwrap_or_else(|| "--".into())
}

pub fn summary(view: &View) -> Vec<Line> {
    let s = &view.snapshot;
    let running = s.processes.iter().filter(|p| p.state == 'R').count();
    vec![
        format!(
            "TOP DOG  up {:.0}m  load {:.2} {:.2} {:.2}  CPU {}%  {} tasks / {} running{}",
            s.uptime_seconds / 60.0,
            s.load[0],
            s.load[1],
            s.load[2],
            percentage(s.cpu_percent),
            s.processes.len(),
            running,
            if view.paused { "  PAUSED" } else { "" }
        )
        .into(),
        format!(
            "RAM {} / {}  available {}   Swap {} / {}   restricted {}",
            bytes(s.memory.total.saturating_sub(s.memory.available)),
            bytes(s.memory.total),
            bytes(s.memory.available),
            bytes(s.memory.swap_total.saturating_sub(s.memory.swap_free)),
            bytes(s.memory.swap_total),
            s.restricted
        )
        .into(),
    ]
}

pub fn process_lines(view: &View, height: usize) -> Vec<Line> {
    let mut out = summary(view);
    out.push(
        format!(
            "Sort {:?}{}  filter '{}'  PID {}  user '{}'",
            view.sort,
            if view.reverse { " reversed" } else { "" },
            display_text(&view.text),
            view.pid
                .map(|p| p.to_string())
                .unwrap_or_else(|| "all".into()),
            display_text(&view.user)
        )
        .into(),
    );
    out.push(
        "     PID USER       PR  NI S    CPU%  MEM%    VIRT     RES     TIME COMMAND"
            .to_string()
            .into(),
    );
    let rows = view.rows();
    let capacity = height.saturating_sub(out.len());
    let index = rows
        .iter()
        .position(|p| Some(p.id()) == view.selected)
        .unwrap_or(0);
    let offset = if capacity > 0 {
        index.saturating_sub(capacity - 1)
    } else {
        0
    };
    for p in rows.iter().skip(offset).take(capacity) {
        let seconds = p.cpu_ticks / view.snapshot.clock_ticks.max(1);
        out.push(Line {
            selected: Some(p.id()) == view.selected,
            text: format!(
                "{:>8} {:<9} {:>3} {:>3} {} {:>7} {:>5.1} {:>7} {:>7} {:>5}:{:02} {}",
                p.id().pid,
                clip(&p.user, 9),
                p.priority,
                p.nice,
                p.state,
                percentage(p.cpu_percent),
                p.memory_percent,
                bytes(p.virtual_bytes),
                bytes(p.resident_bytes),
                seconds / 60,
                seconds % 60,
                display_text(&p.command)
            ),
        });
    }
    if rows.is_empty() && capacity > 0 {
        out.push("No matching processes".to_string().into());
    }
    out
}

pub fn screen(app: &App, width: usize, height: usize) -> Vec<Line> {
    let mut lines = match &app.mode {
        Mode::Help => vec![
            "TOP DOG controls",
            "Up/Down or j/k: select    PageUp/PageDown/Home/End: scroll",
            "P: CPU sort   M: RAM   N: PID   T: CPU time   C: command   r: reverse",
            "/: text filter   p: PID filter   u: user filter   Esc: clear filters",
            "Space: pause display   d: refresh milliseconds",
            "Enter: process details and application targets   o: open target",
            "s: signal   n: renice   y: confirm action   n/Esc: cancel",
            "Targets: Up/Down, Enter to select existing target, Esc to return",
            "q or Ctrl-C: quit   ?: help   Esc: close help",
            "CPU uses 100% per logical core. RSS is process resident memory.",
            "Shared browser processes can map to several tabs and frames.",
        ]
        .into_iter()
        .map(|s| s.to_string().into())
        .collect(),
        Mode::Targets {
            resolution, cursor, ..
        } => {
            let mut out = vec![
                format!(
                    "{} targets for PID {}. Enter: select existing tab, Esc: back",
                    resolution.application, resolution.process.pid
                )
                .into(),
                resolution.summary.clone().into(),
            ];
            let capacity = height.saturating_sub(5) / 2;
            let offset = cursor.saturating_sub(capacity.saturating_sub(1));
            for (i, target) in resolution
                .targets
                .iter()
                .enumerate()
                .skip(offset)
                .take(capacity)
            {
                out.push(Line {
                    text: format!(
                        "{} [{:?}] window {}",
                        display_text(&target.title),
                        target.relationship,
                        display_text(&target.window)
                    ),
                    selected: i == *cursor,
                });
                out.push(Line {
                    text: format!(
                        "  {}",
                        display_text(target.url.as_deref().unwrap_or("URL unavailable"))
                    ),
                    selected: i == *cursor,
                });
            }
            out
        }
        Mode::Confirm {
            process,
            command,
            action,
        } => vec![
            format!("Apply {action:?} to PID {}?", process.pid).into(),
            display_text(command).into(),
            "Press y to confirm or n/Esc to cancel".to_string().into(),
        ],
        _ => {
            let details = app.details.len().min(4);
            let mut out = process_lines(&app.view, height.saturating_sub(3 + details));
            for detail in app.details.iter().take(4) {
                out.push(detail.clone().into());
            }
            out
        }
    };
    lines.truncate(height.saturating_sub(3));
    while lines.len() < height.saturating_sub(3) {
        lines.push(String::new().into());
    }
    lines.push(
        format!(
            "{}{}",
            if app.busy { "[working] " } else { "" },
            display_text(&app.status)
        )
        .into(),
    );
    lines.push(match &app.mode {Mode::Input(kind,text)=>format!("{kind:?}> {text}  [Enter: apply, Esc: cancel]"),_=>format!("Enter inspect | o open | / filter | P CPU | M RAM | Space pause | ? help | q quit   {}ms",app.refresh_ms)}.into());
    lines.truncate(height);
    for line in &mut lines {
        line.text = clip(&line.text, width);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use top_dog_core::{
        config::Config,
        monitor::{Process, Snapshot},
    };
    use top_dog_protocol::ProcessIdentity;
    #[test]
    fn clips_wide_unicode_and_removes_escape_sequences() {
        assert_eq!(clip("日本語", 4), "日本");
        assert_eq!(clip("a\x1b[31mb", 2), "ab");
        assert_eq!(bytes(1024), "1.0K");
    }
    #[test]
    fn renders_selection_and_survives_small_resizes() {
        let mut app = App::new(Config::default());
        app.view.replace(Snapshot {
            processes: vec![Process {
                identity: Some(ProcessIdentity {
                    pid: 42,
                    start_ticks: 1,
                }),
                command: "firefox".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        let lines = screen(&app, 100, 24);
        assert!(
            lines
                .iter()
                .any(|l| l.selected && l.text.contains("firefox"))
        );
        for (w, h) in [(0, 0), (1, 1), (20, 3), (80, 24)] {
            let lines = screen(&app, w, h);
            assert!(lines.len() <= h);
            assert!(
                lines
                    .iter()
                    .all(|l| unicode_width::UnicodeWidthStr::width(l.text.as_str()) <= w)
            );
        }
    }
}
