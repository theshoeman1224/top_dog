use crate::monitor::{Process, Snapshot};
use top_dog_protocol::ProcessIdentity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Cpu,
    Memory,
    Pid,
    Time,
    Command,
}

#[derive(Debug)]
pub struct View {
    pub snapshot: Snapshot,
    pub selected: Option<ProcessIdentity>,
    pub sort: Sort,
    pub reverse: bool,
    pub text: String,
    pub pid: Option<i32>,
    pub user: String,
    pub paused: bool,
}

impl Default for View {
    fn default() -> Self {
        Self {
            snapshot: Snapshot::default(),
            selected: None,
            sort: Sort::Cpu,
            reverse: false,
            text: String::new(),
            pid: None,
            user: String::new(),
            paused: false,
        }
    }
}

impl View {
    pub fn rows(&self) -> Vec<&Process> {
        let text = self.text.to_lowercase();
        let mut rows: Vec<_> = self
            .snapshot
            .processes
            .iter()
            .filter(|p| {
                self.pid.is_none_or(|pid| p.id().pid == pid)
                    && (self.user.is_empty()
                        || self.user == p.user
                        || self.user == p.uid.to_string())
                    && (text.is_empty()
                        || p.command.to_lowercase().contains(&text)
                        || p.name.to_lowercase().contains(&text)
                        || p.id().pid.to_string().contains(&text))
            })
            .collect();
        rows.sort_by(|a, b| {
            let order = match self.sort {
                Sort::Cpu => b
                    .cpu_percent
                    .partial_cmp(&a.cpu_percent)
                    .unwrap_or(std::cmp::Ordering::Equal),
                Sort::Memory => b.resident_bytes.cmp(&a.resident_bytes),
                Sort::Pid => a.id().pid.cmp(&b.id().pid),
                Sort::Time => b.cpu_ticks.cmp(&a.cpu_ticks),
                Sort::Command => a.command.cmp(&b.command),
            }
            .then(a.id().pid.cmp(&b.id().pid));
            if self.reverse { order.reverse() } else { order }
        });
        rows
    }

    pub fn replace(&mut self, snapshot: Snapshot) {
        if self.paused {
            return;
        }
        self.snapshot = snapshot;
        self.reconcile();
    }

    pub fn reconcile(&mut self) {
        let rows = self.rows();
        if !rows.iter().any(|p| Some(p.id()) == self.selected) {
            self.selected = rows.first().map(|p| p.id());
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        let rows = self.rows();
        if rows.is_empty() {
            self.selected = None;
            return;
        }
        let current = rows
            .iter()
            .position(|p| Some(p.id()) == self.selected)
            .unwrap_or(0);
        let next = current.saturating_add_signed(delta).min(rows.len() - 1);
        self.selected = Some(rows[next].id());
    }

    pub fn selected_process(&self) -> Option<&Process> {
        self.snapshot
            .processes
            .iter()
            .find(|p| Some(p.id()) == self.selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn process(pid: i32, start: u64, cpu: f64, resident: u64) -> Process {
        Process {
            identity: Some(ProcessIdentity {
                pid,
                start_ticks: start,
            }),
            cpu_percent: Some(cpu),
            resident_bytes: resident,
            command: format!("command-{pid}"),
            user: "josh".into(),
            uid: 1000,
            ..Default::default()
        }
    }
    #[test]
    fn sorts_filters_and_preserves_identity_across_refreshes() {
        let mut v = View::default();
        v.replace(Snapshot {
            processes: vec![process(1, 10, 20.0, 50), process(2, 20, 80.0, 30)],
            ..Default::default()
        });
        assert_eq!(v.selected.unwrap().pid, 2);
        v.move_selection(1);
        assert_eq!(v.selected.unwrap().pid, 1);
        v.sort = Sort::Memory;
        assert_eq!(v.rows()[0].id().pid, 1);
        v.reverse = true;
        assert_eq!(v.rows()[0].id().pid, 2);
        v.replace(Snapshot {
            processes: vec![process(1, 99, 5.0, 90), process(2, 20, 10.0, 30)],
            ..Default::default()
        });
        assert_ne!(
            v.selected,
            Some(ProcessIdentity {
                pid: 1,
                start_ticks: 10
            })
        );
        v.user = "1000".into();
        v.text = "command-2".into();
        v.reconcile();
        assert_eq!(v.rows().len(), 1);
        assert_eq!(v.selected.unwrap().pid, 2);
        v.pid = Some(1);
        v.reconcile();
        assert!(v.selected.is_none());
    }
    #[test]
    fn pause_freezes_snapshot_and_empty_lists_are_safe() {
        let mut v = View {
            paused: true,
            ..Default::default()
        };
        v.replace(Snapshot {
            processes: vec![process(1, 1, 0.0, 1)],
            ..Default::default()
        });
        assert!(v.rows().is_empty());
        v.move_selection(99);
        assert!(v.selected.is_none());
    }
}
