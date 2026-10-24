use std::{
    fs,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use top_dog_core::monitor;

struct Task(Child);
impl Drop for Task {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn controls_apply_to_a_disposable_task_and_reject_it_after_exit() {
    let mut task = Task(
        Command::new("/usr/bin/sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let process = monitor::identity(task.0.id() as i32).unwrap();
    monitor::renice(process, 5).unwrap();
    let read = || {
        monitor::parse_stat(
            &fs::read_to_string(format!("/proc/{}/stat", process.pid)).unwrap(),
            4096,
        )
        .unwrap()
    };
    assert_eq!(read().nice, 5);
    monitor::signal(process, libc::SIGSTOP).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while read().state != 'T' {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    monitor::signal(process, libc::SIGCONT).unwrap();
    monitor::signal(process, libc::SIGTERM).unwrap();
    assert!(!task.0.wait().unwrap().success());
    assert!(monitor::validate(process).is_err());
    assert!(monitor::signal(process, libc::SIGCONT).is_err());
}
