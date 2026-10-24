use crate::app::{App, Mode};
use top_dog_core::{display_text, view::View};
use unicode_width::UnicodeWidthStr;

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
    if text.width() <= width {
        return text;
    }
    let mut end = 0;
    for (index, c) in text.char_indices() {
        let next = index + c.len_utf8();
        if text[..next].width() > width {
            break;
        }
        end = next;
    }
    text[..end].to_string()
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

#[derive(Clone, Copy)]
enum Alignment {
    Left,
    Right,
}

const PROCESS_COLUMNS: [(&str, usize, Alignment); 10] = [
    ("PID", 8, Alignment::Right),
    ("USER", 9, Alignment::Left),
    ("PR", 4, Alignment::Right),
    ("NI", 3, Alignment::Right),
    ("S", 1, Alignment::Left),
    ("CPU%", 7, Alignment::Right),
    ("MEM%", 5, Alignment::Right),
    ("VIRT", 7, Alignment::Right),
    ("RES", 7, Alignment::Right),
    ("TIME", 8, Alignment::Right),
];

fn process_row(values: [String; 10], command: &str) -> String {
    let mut row = String::new();
    for (value, (_, width, alignment)) in values.iter().zip(PROCESS_COLUMNS) {
        let mut value = display_text(value);
        if value.width() > width {
            value = match alignment {
                Alignment::Left => format!("{}…", clip(&value, width - 1)),
                // A shortened number could look like a valid, smaller measurement.
                Alignment::Right => "#".repeat(width),
            };
        }
        let padding = " ".repeat(width - value.width());
        match alignment {
            Alignment::Left => {
                row.push_str(&value);
                row.push_str(&padding);
            }
            Alignment::Right => {
                row.push_str(&padding);
                row.push_str(&value);
            }
        }
        row.push(' ');
    }
    row.push_str(&display_text(command));
    row
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
        process_row(
            PROCESS_COLUMNS.map(|(label, _, _)| label.to_string()),
            "COMMAND",
        )
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
            text: process_row(
                [
                    p.id().pid.to_string(),
                    p.user.clone(),
                    p.priority.to_string(),
                    p.nice.to_string(),
                    p.state.to_string(),
                    percentage(p.cpu_percent),
                    format!("{:.1}", p.memory_percent),
                    bytes(p.virtual_bytes),
                    bytes(p.resident_bytes),
                    format!("{}:{:02}", seconds / 60, seconds % 60),
                ],
                &p.command,
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
        assert_eq!(clip("❤\u{fe0f}", 1), "❤");
        assert_eq!(clip("❤\u{fe0f}", 2), "❤\u{fe0f}");
        assert_eq!(clip("👩‍💻x", 2), "👩‍💻");
        assert_eq!(bytes(1024), "1.0K");
    }
    fn table_fixture(user: &str) -> View {
        let mut view = View::default();
        view.replace(Snapshot {
            clock_ticks: 100,
            processes: vec![Process {
                identity: Some(ProcessIdentity {
                    pid: 42,
                    start_ticks: 1,
                }),
                user: user.into(),
                priority: 20,
                nice: -5,
                state: 'R',
                cpu_percent: Some(12.3),
                memory_percent: 4.5,
                virtual_bytes: 1024,
                resident_bytes: 512,
                cpu_ticks: 12_300,
                command: "command-marker".into(),
                ..Default::default()
            }],
            ..Default::default()
        });
        view
    }

    fn display_column(text: &str, needle: &str) -> usize {
        unicode_width::UnicodeWidthStr::width(&text[..text.find(needle).unwrap()])
    }

    #[test]
    fn process_columns_align_with_header() {
        let lines = process_lines(&table_fixture("user"), 10);
        let header = &lines[3].text;
        let row = &lines[4].text;
        for (label, value) in [
            ("PID", "42"),
            ("PR", "20"),
            ("NI", "-5"),
            ("CPU%", "12.3"),
            ("MEM%", "4.5"),
            ("VIRT", "1.0K"),
            ("RES", "512B"),
            ("TIME", "2:03"),
        ] {
            assert_eq!(
                display_column(row, value) + value.len(),
                display_column(header, label) + label.len(),
                "{label}: {row}"
            );
        }
        assert_eq!(display_column(row, "user"), display_column(header, "USER"));
        assert_eq!(display_column(row, "R"), display_column(header, " S ") + 1);
        assert_eq!(
            display_column(row, "command-marker"),
            display_column(header, "COMMAND")
        );
    }

    #[test]
    fn process_columns_stay_bounded_with_long_and_unicode_values() {
        let reference = process_lines(&table_fixture("user"), 10);
        let command_column = display_column(&reference[3].text, "COMMAND");
        for user in [
            "very-long-username",
            "日本語の利用者",
            "e\u{301}e\u{301}e\u{301}",
            "❤\u{fe0f}".repeat(8).as_str(),
        ] {
            let lines = process_lines(&table_fixture(user), 10);
            assert_eq!(
                display_column(&lines[4].text, "command-marker"),
                command_column,
                "user {user:?}"
            );
        }
        for field in [
            "pid", "priority", "nice", "cpu", "memory", "virtual", "resident", "time",
        ] {
            let mut view = table_fixture("user");
            let process = &mut view.snapshot.processes[0];
            match field {
                "pid" => process.identity.as_mut().unwrap().pid = i32::MAX,
                "priority" => process.priority = i64::MIN,
                "nice" => process.nice = i64::MIN,
                "cpu" => process.cpu_percent = Some(1e20),
                "memory" => process.memory_percent = 1e20,
                "virtual" => process.virtual_bytes = u64::MAX,
                "resident" => process.resident_bytes = u64::MAX,
                "time" => process.cpu_ticks = u64::MAX,
                _ => unreachable!(),
            }
            let lines = process_lines(&view, 10);
            assert_eq!(
                display_column(&lines[4].text, "command-marker"),
                command_column,
                "overflow in {field}"
            );
            assert!(
                lines[4].text.contains('#'),
                "missing overflow marker for {field}"
            );
        }
    }

    #[test]
    fn screen_bounds_long_unicode_commands_after_resize() {
        let mut app = App::new(Config::default());
        app.view = table_fixture("日本語の利用者");
        app.view.snapshot.processes[0].command = "❤\u{fe0f}👩‍💻日本語 e\u{301}".repeat(100);
        for width in [0, 1, 2, 40, 68, 69, 70, 71, 80, 100, 120] {
            let lines = screen(&app, width, 24);
            assert!(lines.iter().any(|line| line.selected));
            assert!(
                lines.iter().all(|line| line.text.width() <= width),
                "width {width}"
            );
        }
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
