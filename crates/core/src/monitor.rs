//! Linux sampling and process control. No terminal or plugin knowledge lives here.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::Path,
    time::Duration,
};
use top_dog_protocol::ProcessIdentity;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Process {
    pub identity: Option<ProcessIdentity>,
    pub parent: i32,
    pub uid: u32,
    pub user: String,
    pub priority: i64,
    pub nice: i64,
    pub state: char,
    pub threads: u64,
    pub cpu_ticks: u64,
    pub cpu_percent: Option<f64>,
    pub memory_percent: f64,
    pub virtual_bytes: u64,
    pub resident_bytes: u64,
    pub command: String,
    pub name: String,
    pub executable: Option<String>,
}

impl Process {
    pub fn id(&self) -> ProcessIdentity {
        self.identity.expect("collected processes have an identity")
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Memory {
    pub total: u64,
    pub available: u64,
    pub swap_total: u64,
    pub swap_free: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub uptime_seconds: f64,
    pub load: [f64; 3],
    pub memory: Memory,
    pub cpu_percent: Option<f64>,
    pub processes: Vec<Process>,
    pub restricted: usize,
    pub clock_ticks: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CpuCounters {
    pub total: u64,
    pub idle: u64,
}

#[derive(Debug, Default)]
pub struct Sampler {
    previous_processes: HashMap<ProcessIdentity, u64>,
    previous_cpu: Option<CpuCounters>,
}

impl Sampler {
    pub fn collect(&mut self, elapsed: Duration) -> Result<Snapshot, String> {
        let ticks = clock_ticks();
        let page_size = page_size();
        let memory =
            parse_memory(&fs::read_to_string("/proc/meminfo").map_err(|e| e.to_string())?)?;
        let counters = parse_cpu(&fs::read_to_string("/proc/stat").map_err(|e| e.to_string())?)?;
        let cpu_percent = self.previous_cpu.and_then(|old| system_cpu(old, counters));
        self.previous_cpu = Some(counters);
        let uptime_seconds = fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|s| s.split_whitespace().next()?.parse().ok())
            .unwrap_or(0.0);
        let load_text = fs::read_to_string("/proc/loadavg").map_err(|e| e.to_string())?;
        let mut load = [0.0; 3];
        for (dest, value) in load.iter_mut().zip(load_text.split_whitespace()) {
            *dest = value.parse().unwrap_or(0.0);
        }
        let users = read_users();
        let mut processes = Vec::new();
        let mut next = HashMap::new();
        let mut restricted = 0;
        for entry in fs::read_dir("/proc").map_err(|e| e.to_string())?.flatten() {
            if entry.file_name().to_string_lossy().parse::<i32>().is_err() {
                continue;
            }
            match read_process(&entry.path(), page_size, &users) {
                Ok(mut p) => {
                    p.cpu_percent = self
                        .previous_processes
                        .get(&p.id())
                        .and_then(|old| process_cpu(*old, p.cpu_ticks, elapsed, ticks));
                    p.memory_percent = if memory.total == 0 {
                        0.0
                    } else {
                        p.resident_bytes as f64 * 100.0 / memory.total as f64
                    };
                    next.insert(p.id(), p.cpu_ticks);
                    processes.push(p);
                }
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => restricted += 1,
                Err(_) => {} // A process may exit between any two reads.
            }
        }
        self.previous_processes = next;
        Ok(Snapshot {
            uptime_seconds,
            load,
            memory,
            cpu_percent,
            processes,
            restricted,
            clock_ticks: ticks,
        })
    }
}

pub fn clock_ticks() -> u64 {
    // sysconf has no pointer arguments and does not mutate process state.
    let n = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if n > 0 { n as u64 } else { 100 }
}

fn page_size() -> u64 {
    // See clock_ticks; a positive result is required before conversion.
    let n = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if n > 0 { n as u64 } else { 4096 }
}

pub fn process_cpu(old: u64, new: u64, elapsed: Duration, ticks: u64) -> Option<f64> {
    if elapsed.is_zero() || ticks == 0 {
        return None;
    }
    Some(new.checked_sub(old)? as f64 / ticks as f64 / elapsed.as_secs_f64() * 100.0)
}

pub fn system_cpu(old: CpuCounters, new: CpuCounters) -> Option<f64> {
    let total = new.total.checked_sub(old.total)?;
    if total == 0 {
        return None;
    }
    let idle = new.idle.checked_sub(old.idle)?;
    Some(total.saturating_sub(idle) as f64 / total as f64 * 100.0)
}

pub fn parse_cpu(text: &str) -> Result<CpuCounters, String> {
    let line = text
        .lines()
        .find(|l| l.starts_with("cpu "))
        .ok_or("missing aggregate CPU counters")?;
    let nums = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .map(|v| {
            v.parse::<u64>()
                .map_err(|_| "invalid CPU counter".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if nums.len() < 4 {
        return Err("incomplete CPU counters".into());
    }
    Ok(CpuCounters {
        total: nums.iter().sum(),
        idle: nums[3] + nums.get(4).copied().unwrap_or(0),
    })
}

pub fn parse_memory(text: &str) -> Result<Memory, String> {
    let mut fields = HashMap::new();
    for line in text.lines() {
        if let Some((key, rest)) = line.split_once(':')
            && let Some(value) = rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<u64>().ok())
        {
            fields.insert(key, value.saturating_mul(1024));
        }
    }
    let total = *fields.get("MemTotal").ok_or("missing MemTotal")?;
    let get = |k| fields.get(k).copied().unwrap_or(0);
    let fallback = get("MemFree")
        .saturating_add(get("Buffers"))
        .saturating_add(get("Cached"))
        .saturating_add(get("SReclaimable"))
        .saturating_sub(get("Shmem"));
    Ok(Memory {
        total,
        available: fields
            .get("MemAvailable")
            .copied()
            .unwrap_or(fallback)
            .min(total),
        swap_total: get("SwapTotal"),
        swap_free: get("SwapFree"),
    })
}

pub fn parse_stat(text: &str, page_size: u64) -> Result<Process, String> {
    let start = text.find('(').ok_or("missing command in stat")?;
    let end = text.rfind(')').ok_or("unterminated command in stat")?;
    if end <= start {
        return Err("invalid command in stat".into());
    }
    let pid = text[..start]
        .trim()
        .parse::<i32>()
        .map_err(|_| "invalid PID")?;
    if pid <= 0 {
        return Err("invalid PID".into());
    }
    let f: Vec<_> = text[end + 1..].split_whitespace().collect();
    if f.len() < 22 {
        return Err("incomplete process stat".into());
    }
    let n = |i: usize| {
        f[i].parse::<u64>()
            .map_err(|_| "invalid unsigned stat field".to_string())
    };
    let signed = |i: usize| {
        f[i].parse::<i64>()
            .map_err(|_| "invalid signed stat field".to_string())
    };
    let name = text[start + 1..end].to_string();
    Ok(Process {
        identity: Some(ProcessIdentity {
            pid,
            start_ticks: n(19)?,
        }),
        parent: i32::try_from(signed(1)?).map_err(|_| "invalid parent PID")?,
        priority: signed(15)?,
        nice: signed(16)?,
        state: f[0].chars().next().ok_or("missing process state")?,
        threads: n(17)?,
        cpu_ticks: n(11)?.saturating_add(n(12)?),
        virtual_bytes: n(20)?,
        resident_bytes: (signed(21)?.max(0) as u64).saturating_mul(page_size),
        command: name.clone(),
        name,
        ..Default::default()
    })
}

fn read_users() -> HashMap<u32, String> {
    fs::read_to_string("/etc/passwd")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split(':').collect();
            Some((fields.get(2)?.parse().ok()?, fields.first()?.to_string()))
        })
        .collect()
}

fn read_process(
    path: &Path,
    page_size: u64,
    users: &HashMap<u32, String>,
) -> std::io::Result<Process> {
    let stat = fs::read_to_string(path.join("stat"))?;
    let mut p = parse_stat(&stat, page_size).map_err(std::io::Error::other)?;
    let status = fs::read_to_string(path.join("status"))?;
    p.uid = status
        .lines()
        .find_map(|l| {
            l.strip_prefix("Uid:")?
                .split_whitespace()
                .nth(1)? // top's USER column uses the effective UID.
                .parse()
                .ok()
        })
        .ok_or_else(|| std::io::Error::other("missing Uid"))?;
    p.user = users
        .get(&p.uid)
        .cloned()
        .unwrap_or_else(|| p.uid.to_string());
    p.executable = fs::read_link(path.join("exe"))
        .ok()
        .map(|p| p.to_string_lossy().into_owned());
    if let Ok(bytes) = fs::read(path.join("cmdline")) {
        let args = String::from_utf8_lossy(&bytes)
            .trim_end_matches('\0')
            .replace('\0', " ");
        if !args.is_empty() {
            p.command = args;
        }
    }
    // Ensure values read from different files still belong to the same process.
    let current = parse_stat(&fs::read_to_string(path.join("stat"))?, page_size)
        .map_err(std::io::Error::other)?;
    if current.identity != p.identity {
        return Err(std::io::Error::other("process replaced while sampling"));
    }
    Ok(p)
}

pub fn identity(pid: i32) -> Result<ProcessIdentity, String> {
    if pid <= 0 {
        return Err("PID must be positive".into());
    }
    Ok(parse_stat(
        &fs::read_to_string(format!("/proc/{pid}/stat")).map_err(|e| e.to_string())?,
        page_size(),
    )?
    .id())
}

pub fn validate(process: ProcessIdentity) -> Result<(), String> {
    if identity(process.pid)? != process {
        return Err("process has exited or PID was reused".into());
    }
    Ok(())
}

pub fn details(process: ProcessIdentity) -> Result<String, String> {
    validate(process)?;
    let path = format!("/proc/{}", process.pid);
    let link = |name| {
        fs::read_link(format!("{path}/{name}"))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "unavailable".into())
    };
    let result = format!(
        "Executable: {}\nWorking directory: {}\nStart ticks: {}\nPID namespace: {}",
        link("exe"),
        link("cwd"),
        process.start_ticks,
        link("ns/pid")
    );
    validate(process)?;
    Ok(result)
}

pub fn signal(process: ProcessIdentity, signal: i32) -> Result<(), String> {
    if ![
        libc::SIGTERM,
        libc::SIGKILL,
        libc::SIGSTOP,
        libc::SIGCONT,
        libc::SIGHUP,
        libc::SIGINT,
    ]
    .contains(&signal)
    {
        return Err("unsupported signal".into());
    }
    // pidfd pins the task, so PID reuse cannot redirect a signal to another task.
    let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, process.pid, 0) };
    if raw < 0 {
        return Err(format!(
            "cannot open process handle: {}",
            std::io::Error::last_os_error()
        ));
    }
    // The successful syscall returned a newly owned descriptor.
    let fd = unsafe { OwnedFd::from_raw_fd(raw as i32) };
    validate(process)?;
    // Null siginfo requests normal kill-like delivery to the pinned task.
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            fd.as_raw_fd(),
            signal,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if result < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

pub fn renice(process: ProcessIdentity, nice: i32) -> Result<(), String> {
    if !(-20..=19).contains(&nice) {
        return Err("nice value must be -20 through 19".into());
    }
    validate(process)?;
    // Linux setpriority accepts a PID rather than a pidfd; validation is best effort.
    let result = unsafe { libc::setpriority(libc::PRIO_PROCESS, process.pid as u32, nice) };
    if result < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_parentheses_spaces_and_signed_stat_fields() {
        let text = "42 (weird ) name) S 1 0 0 0 0 0 0 0 0 0 12 8 0 0 -5 -10 3 0 1234 8192 2";
        let p = parse_stat(text, 4096).unwrap();
        assert_eq!(p.name, "weird ) name");
        assert_eq!(p.cpu_ticks, 20);
        assert_eq!(p.nice, -10);
        assert_eq!(p.priority, -5);
        assert_eq!(p.id().start_ticks, 1234);
        assert_eq!(p.resident_bytes, 8192);
        assert!(parse_stat("1 (broken)", 4096).is_err());
    }
    #[test]
    fn cpu_is_per_core_and_handles_counter_resets() {
        assert_eq!(
            process_cpu(100, 500, Duration::from_secs(2), 100),
            Some(200.0)
        );
        assert_eq!(process_cpu(100, 90, Duration::from_secs(1), 100), None);
        assert_eq!(process_cpu(0, 1, Duration::ZERO, 100), None);
        let old = CpuCounters {
            total: 100,
            idle: 20,
        };
        assert_eq!(
            system_cpu(
                old,
                CpuCounters {
                    total: 200,
                    idle: 70
                }
            ),
            Some(50.0)
        );
        assert_eq!(system_cpu(old, old), None);
    }
    #[test]
    fn guest_counters_are_not_counted_twice() {
        let cpu = parse_cpu("cpu 10 20 30 40 5 6 7 8 100 200\ncpu0 1").unwrap();
        assert_eq!(cpu.total, 126);
        assert_eq!(cpu.idle, 45);
    }
    #[test]
    fn memory_uses_kibibytes_and_available_fallback() {
        let m = parse_memory("MemTotal: 100 kB\nMemFree: 10 kB\nBuffers: 5 kB\nCached: 20 kB\nSReclaimable: 3 kB\nShmem: 2 kB\nSwapTotal: 50 kB\nSwapFree: 40 kB").unwrap();
        assert_eq!(m.total, 102400);
        assert_eq!(m.available, 36 * 1024);
        assert_eq!(m.swap_total - m.swap_free, 10 * 1024);
        assert!(parse_memory("garbage").is_err());
    }
    #[test]
    fn stale_identity_cannot_receive_actions() {
        let mut id = identity(std::process::id() as i32).unwrap();
        id.start_ticks += 1;
        assert!(validate(id).is_err());
        assert!(signal(id, libc::SIGCONT).is_err());
        assert!(renice(id, 0).is_err());
        assert!(identity(0).is_err());
    }
    #[test]
    fn live_sample_has_no_initial_cpu_and_recognizes_self() {
        let mut sampler = Sampler::default();
        let sample = sampler.collect(Duration::ZERO).unwrap();
        assert!(sample.memory.total > 0);
        let me = sample
            .processes
            .iter()
            .find(|p| p.id().pid == std::process::id() as i32)
            .unwrap();
        assert_eq!(me.cpu_percent, None);
        assert!(details(me.id()).unwrap().contains("Executable:"));
    }
}
