use crate::{app::App, render};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event, KeyEventKind},
    execute, queue,
    style::{Attribute, Print, SetAttribute},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{
    io::{self, IsTerminal, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};
use top_dog_core::monitor::{Sampler, Snapshot};

static TERMINATED: AtomicBool = AtomicBool::new(false);
extern "C" fn terminate(_: i32) {
    TERMINATED.store(true, Ordering::Relaxed);
}

struct Terminal;
impl Terminal {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let guard = Self;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Ok(guard)
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        restore();
    }
}
pub fn restore() {
    let _ = terminal::disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        SetAttribute(Attribute::Reset),
        Show,
        LeaveAlternateScreen
    );
}

enum Control {
    Refresh(u64),
    Stop,
}
struct Collector {
    tx: Sender<Control>,
    rx: Receiver<Result<Snapshot, String>>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Collector {
    fn start(refresh: u64) -> Self {
        let (control_tx, control_rx) = mpsc::channel();
        let (tx, rx) = mpsc::sync_channel(1);
        let handle = thread::spawn(move || {
            let mut sampler = Sampler::default();
            let mut last = None;
            let mut refresh = refresh;
            loop {
                let now = Instant::now();
                let elapsed = last.map(|v| now.duration_since(v)).unwrap_or_default();
                last = Some(now);
                let _ = tx.try_send(sampler.collect(elapsed));
                match control_rx.recv_timeout(Duration::from_millis(refresh)) {
                    Ok(Control::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Ok(Control::Refresh(n)) => refresh = n,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
        });
        Self {
            tx: control_tx,
            rx,
            thread: Some(handle),
        }
    }
}
impl Drop for Collector {
    fn drop(&mut self) {
        let _ = self.tx.send(Control::Stop);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

pub fn run(mut app: App) -> Result<(), String> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(
            "Interactive mode needs a terminal. Use --batch or --json for redirected output."
                .into(),
        );
    }
    let old_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        old_hook(info);
    }));
    // Handler only writes a lock-free flag. The main loop performs terminal cleanup.
    let previous =
        unsafe { libc::signal(libc::SIGTERM, terminate as *const () as libc::sighandler_t) };
    let result = run_loop(&mut app);
    // Restore the embedding process's previous signal disposition.
    unsafe {
        libc::signal(libc::SIGTERM, previous);
    }
    result.map_err(|e| e.to_string())
}

fn run_loop(app: &mut App) -> io::Result<()> {
    let _terminal = Terminal::enter()?;
    let collector = Collector::start(app.refresh_ms);
    let mut refresh = app.refresh_ms;
    let mut redraw = true;
    while !app.quit && !TERMINATED.load(Ordering::Relaxed) {
        redraw |= app.poll();
        while let Ok(sample) = collector.rx.try_recv() {
            match sample {
                Ok(snapshot) => {
                    redraw |= !app.view.paused;
                    app.update_snapshot(snapshot);
                }
                Err(error) => {
                    app.status = format!("Sampling failed: {error}");
                    redraw = true;
                }
            }
        }
        if refresh != app.refresh_ms {
            refresh = app.refresh_ms;
            let _ = collector.tx.send(Control::Refresh(refresh));
        }
        if redraw {
            let (width, height) = terminal::size()?;
            let lines = render::screen(app, width as usize, height as usize);
            let mut stdout = io::stdout().lock();
            queue!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;
            for (index, line) in lines.iter().enumerate() {
                queue!(
                    stdout,
                    MoveTo(0, index as u16),
                    SetAttribute(if line.selected {
                        Attribute::Reverse
                    } else {
                        Attribute::Reset
                    }),
                    Print(&line.text),
                    SetAttribute(Attribute::Reset)
                )?;
            }
            stdout.flush()?;
            redraw = false;
        }
        if event::poll(Duration::from_millis(80))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    app.key(key);
                    redraw = true;
                }
                Event::Resize(_, _) => redraw = true,
                _ => {}
            }
        }
    }
    Ok(())
}
