use crate::{app::App, render, terminal};
use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
use top_dog_core::{
    config::Config,
    monitor::Sampler,
    plugin,
    view::{Sort, View},
};
use top_dog_protocol::{Operation, Reply};

pub const HELP: &str = "top-dog [OPTIONS]

Linux process monitor with application-specific PID inspection and navigation.

  --config PATH         Read this configuration instead of the XDG default
  --refresh-ms N        Sampling interval, 100 to 60000 (default 1000)
  --pid PID             Filter by PID
  --user NAME_OR_UID    Filter by user
  --filter TEXT         Filter command, name, or PID
  --sort FIELD          cpu, memory, pid, time, command (default cpu)
  --reverse             Reverse sorting
  --batch               Print process tables without a terminal
  --json                Print one JSON snapshot per line; implies --batch
  --iterations N        Number of batch snapshots (default 1)
  --doctor              Describe and diagnose all configured plugins
  --check-plugin NAME   Validate one configured plugin's describe/diagnose contract
  --help                Show help
  --version             Show version

In the monitor press ? for controls. Press Enter to inspect or o to select an
existing application target. Configure the Firefox plugin before tab navigation.";

#[derive(Default, Debug)]
pub struct Options {
    pub config: Option<PathBuf>,
    pub refresh: Option<u64>,
    pub pid: Option<i32>,
    pub user: String,
    pub filter: String,
    pub sort: Option<Sort>,
    pub reverse: bool,
    pub batch: bool,
    pub json: bool,
    pub iterations: Option<usize>,
    pub doctor: bool,
    pub check: Option<String>,
    pub help: bool,
    pub version: bool,
}

impl Options {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut options = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let value = |args: &mut dyn Iterator<Item = String>| -> Result<String, String> {
                args.next().ok_or_else(|| format!("{arg} needs a value"))
            };
            match arg.as_str() {
                "--config" => options.config = Some(value(&mut args)?.into()),
                "--refresh-ms" => {
                    let n = value(&mut args)?
                        .parse::<u64>()
                        .map_err(|_| "invalid refresh interval")?;
                    if !(100..=60000).contains(&n) {
                        return Err("refresh must be 100 to 60000 milliseconds".into());
                    }
                    options.refresh = Some(n);
                }
                "--pid" => {
                    let n = value(&mut args)?
                        .parse::<i32>()
                        .map_err(|_| "invalid PID")?;
                    if n <= 0 {
                        return Err("PID must be positive".into());
                    }
                    options.pid = Some(n);
                }
                "--user" => options.user = value(&mut args)?,
                "--filter" => options.filter = value(&mut args)?,
                "--sort" => {
                    options.sort = Some(match value(&mut args)?.as_str() {
                        "cpu" => Sort::Cpu,
                        "memory" => Sort::Memory,
                        "pid" => Sort::Pid,
                        "time" => Sort::Time,
                        "command" => Sort::Command,
                        _ => return Err("sort must be cpu, memory, pid, time, or command".into()),
                    })
                }
                "--reverse" => options.reverse = true,
                "--batch" => options.batch = true,
                "--json" => {
                    options.json = true;
                    options.batch = true;
                }
                "--iterations" => {
                    let n = value(&mut args)?
                        .parse::<usize>()
                        .map_err(|_| "invalid iterations")?;
                    if n == 0 {
                        return Err("iterations must be positive".into());
                    }
                    options.iterations = Some(n);
                }
                "--doctor" => options.doctor = true,
                "--check-plugin" => options.check = Some(value(&mut args)?),
                "--help" | "-h" => options.help = true,
                "--version" | "-V" => options.version = true,
                _ => return Err(format!("unknown option: {arg}")),
            }
        }
        if options.iterations.is_some() && !options.batch {
            return Err("--iterations requires --batch or --json".into());
        }
        Ok(options)
    }
}

pub fn run(options: Options) -> Result<(), String> {
    if options.help {
        println!("{HELP}");
        return Ok(());
    }
    if options.version {
        println!("top-dog {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let mut config = Config::load(options.config.as_deref())?;
    if let Some(refresh) = options.refresh {
        config.refresh_ms = refresh;
    }
    if options.doctor || options.check.is_some() {
        return doctor(&config, options.check.as_deref());
    }
    let mut app = App::new(config);
    app.view.pid = options.pid;
    app.view.user = options.user;
    app.view.text = options.filter;
    app.view.reverse = options.reverse;
    if let Some(sort) = options.sort {
        app.view.sort = sort;
    }
    if options.batch {
        batch(
            &mut app.view,
            app.refresh_ms,
            options.iterations.unwrap_or(1),
            options.json,
        )
    } else {
        terminal::run(app)
    }
}

fn batch(view: &mut View, refresh: u64, iterations: usize, json: bool) -> Result<(), String> {
    let mut sampler = Sampler::default();
    let mut previous = None;
    for index in 0..iterations {
        if index > 0 {
            thread::sleep(Duration::from_millis(refresh));
        }
        let now = Instant::now();
        let elapsed = previous.map(|p| now.duration_since(p)).unwrap_or_default();
        previous = Some(now);
        view.replace(sampler.collect(elapsed)?);
        if json {
            let mut snapshot = view.snapshot.clone();
            snapshot.processes = view.rows().into_iter().cloned().collect();
            println!(
                "{}",
                serde_json::to_string(&snapshot).map_err(|e| e.to_string())?
            );
        } else {
            for line in render::process_lines(view, view.snapshot.processes.len() + 5) {
                println!("{}", line.text);
            }
        }
    }
    Ok(())
}

fn doctor(config: &Config, filter: Option<&str>) -> Result<(), String> {
    let plugins: Vec<_> = config
        .plugins
        .iter()
        .filter(|p| filter.is_none_or(|f| p.name == f))
        .collect();
    if plugins.is_empty() {
        return Err(
            "No matching plugins configured. See docs/firefox.md and examples/config.toml.".into(),
        );
    }
    let mut failed = false;
    for configured in plugins {
        match plugin::call(configured, Operation::Describe, config.timeout()) {
            Ok(Reply::Description { name, version, .. }) => println!(
                "{}: {} {}",
                configured.name,
                top_dog_core::display_text(&name),
                top_dog_core::display_text(&version)
            ),
            Err(error) => {
                failed = true;
                println!(
                    "{}: {}",
                    top_dog_core::display_text(&configured.name),
                    top_dog_core::display_text(&error)
                );
                continue;
            }
            _ => {
                failed = true;
                continue;
            }
        }
        match plugin::call(configured, Operation::Diagnose, config.timeout()) {
            Ok(Reply::Diagnosis { available, message }) => {
                failed |= !available;
                for line in message.lines() {
                    println!("  {}", top_dog_core::display_text(line));
                }
            }
            Err(error) => {
                failed = true;
                println!("  {}", top_dog_core::display_text(&error));
            }
            _ => failed = true,
        }
    }
    if failed {
        Err("One or more plugin checks failed".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str]) -> Result<Options, String> {
        Options::parse(args.iter().map(|s| s.to_string()))
    }
    #[test]
    fn validates_cli_and_selects_json_batch() {
        let o = parse(&[
            "--json",
            "--iterations",
            "2",
            "--pid",
            "12",
            "--sort",
            "memory",
        ])
        .unwrap();
        assert!(o.batch && o.json);
        assert_eq!(o.pid, Some(12));
        assert_eq!(o.sort, Some(Sort::Memory));
        for args in [
            &["--pid", "0"][..],
            &["--refresh-ms", "0"],
            &["--sort", "banana"],
            &["--iterations", "2"],
            &["--config"],
            &["--oops"],
        ] {
            assert!(parse(args).is_err());
        }
    }
}
