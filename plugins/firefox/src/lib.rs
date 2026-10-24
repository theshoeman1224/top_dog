pub mod marionette;
use marionette::{Client, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{fs, net::SocketAddr};
use top_dog_core::{
    activation::{DesktopActivation, NativeDesktop},
    monitor,
};
use top_dog_protocol::{
    Activation, ErrorCode, FocusResult, Operation, PluginError, ProcessIdentity, Reply, Resolution,
    Target,
};

pub const BRIDGE: &str = include_str!("bridge.js");

#[derive(Default)]
pub struct Firefox {
    pub endpoints: Vec<SocketAddr>,
    pub include_private: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserReply {
    application_pid: i32,
    browser_version: String,
    #[serde(default)]
    targets: Vec<Target>,
    activation: Option<BrowserActivation>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserActivation {
    selected: bool,
    focus_requested: bool,
}

#[derive(Serialize, Deserialize)]
struct TargetToken {
    browser: ProcessIdentity,
    endpoint: SocketAddr,
    target: String,
}

impl Firefox {
    pub fn handle(&self, operation: Operation) -> Result<Reply> {
        match operation {
            Operation::Describe => Ok(Reply::Description {
                name: "firefox".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                capabilities: vec!["diagnose".into(), "resolve".into(), "activate".into()],
            }),
            Operation::Diagnose => {
                if self.endpoints.is_empty() {
                    return Ok(Reply::Diagnosis { available: false, message: "No Firefox endpoints configured. Add --endpoint 127.0.0.1:2828 to the plugin arguments.".into() });
                }
                let mut messages = Vec::new();
                let mut available = false;
                for endpoint in &self.endpoints {
                    let result = self.inspect(*endpoint);
                    match result {
                        Ok((_, info)) => {
                            available = true;
                            messages.push(format!(
                                "{endpoint}: Firefox {} PID {} ready",
                                info.browser_version, info.application_pid
                            ));
                        }
                        Err(error) => messages.push(format!("{endpoint}: {error}")),
                    }
                }
                Ok(Reply::Diagnosis {
                    available,
                    message: messages.join("\n"),
                })
            }
            Operation::Resolve { process } => self.resolve(process).map(Reply::Resolution),
            Operation::Activate { process, target_id } => {
                self.activate(process, &target_id).map(Reply::Activation)
            }
        }
    }

    fn inspect(&self, endpoint: SocketAddr) -> Result<(Client<std::net::TcpStream>, BrowserReply)> {
        let mut client = Client::connect(endpoint)?;
        let info = self.execute(&mut client, json!({"action":"diagnose"}))?;
        same_namespace(info.application_pid)?;
        Ok((client, info))
    }

    fn execute(
        &self,
        client: &mut Client<std::net::TcpStream>,
        options: serde_json::Value,
    ) -> Result<BrowserReply> {
        serde_json::from_value(client.execute(BRIDGE, options)?).map_err(|_| {
            PluginError::new(
                ErrorCode::Unsupported,
                "Firefox bridge returned an incompatible result",
            )
        })
    }

    fn resolve(&self, process: ProcessIdentity) -> Result<Resolution> {
        validate(process)?;
        let mut errors = Vec::new();
        for endpoint in &self.endpoints {
            let (mut client, info) = match self.inspect(*endpoint) {
                Ok(value) => value,
                Err(e) => {
                    errors.push(e.to_string());
                    continue;
                }
            };
            let browser = match owner(process, info.application_pid) {
                Ok(id) => id,
                Err(e) => {
                    errors.push(e.to_string());
                    continue;
                }
            };
            let info = self.execute(
                &mut client,
                json!({"action":"resolve","pid":process.pid,"includePrivate":self.include_private}),
            )?;
            validate(process)?;
            validate(browser)?;
            let mut targets = info.targets;
            for target in &mut targets {
                target.id = serde_json::to_string(&TargetToken {
                    browser,
                    endpoint: *endpoint,
                    target: target.id.clone(),
                })
                .map_err(|_| PluginError::new(ErrorCode::Internal, "cannot encode target"))?;
            }
            let summary = if targets.is_empty() {
                "No visible tab maps to this process. It may be the browser, GPU, network, extension, worker, or another helper process. Private tabs are excluded unless enabled.".into()
            } else {
                format!(
                    "{} related tab(s). A process may be shared; its CPU and memory totals cannot identify an individual tab's share.",
                    targets.len()
                )
            };
            return Ok(Resolution {
                process,
                application: "Firefox".into(),
                summary,
                targets,
            });
        }
        Err(PluginError::new(
            ErrorCode::Unavailable,
            if errors.is_empty() {
                "No Firefox endpoints configured; run top-dog --doctor after configuring the plugin.".into()
            } else {
                format!(
                    "No configured browser owns this process. {}",
                    errors.join("; ")
                )
            },
        ))
    }

    fn activate(&self, process: ProcessIdentity, target_id: &str) -> Result<Activation> {
        validate(process)?;
        let token: TargetToken = serde_json::from_str(target_id).map_err(|_| {
            PluginError::new(ErrorCode::InvalidRequest, "invalid target identifier")
        })?;
        if !self.endpoints.contains(&token.endpoint) {
            return Err(PluginError::new(
                ErrorCode::InvalidRequest,
                "target endpoint is not configured",
            ));
        }
        validate(token.browser)?;
        let (mut client, info) = self.inspect(token.endpoint)?;
        if owner(process, info.application_pid)? != token.browser {
            return Err(PluginError::new(
                ErrorCode::StaleTarget,
                "browser instance has changed",
            ));
        }
        validate(process)?;
        let info = self.execute(&mut client, json!({"action":"activate","pid":process.pid,"includePrivate":self.include_private,"target":token.target}))?;
        let activation = info.activation.ok_or_else(|| {
            PluginError::new(
                ErrorCode::StaleTarget,
                "tab or frame closed, navigated, or changed process; resolve this process again",
            )
        })?;
        let desktop = NativeDesktop::detect();
        let focus = desktop.request_focus(|| {
            let reply = self.execute(&mut client, json!({"action":"focus","pid":process.pid,"includePrivate":self.include_private,"target":token.target})).map_err(|e|e.to_string())?;
            Ok(reply.activation.is_some_and(|a| a.selected && a.focus_requested))
        }).unwrap_or(FocusResult::Unsupported);
        let message = match (activation.selected, focus, desktop.wayland) {
            (false, _, _) => "Firefox did not confirm selection of the existing tab.",
            (true, FocusResult::Requested, true) => {
                "Existing tab selected. Window focus requested; your Wayland desktop may require you to switch to Firefox."
            }
            (true, FocusResult::Requested, false) => {
                "Existing tab selected. Window focus requested; foreground focus cannot be verified."
            }
            (true, _, _) => {
                "Existing tab selected. Window focus could not be requested; switch to Firefox to view it."
            }
        };
        Ok(Activation {
            selected: activation.selected,
            focus,
            message: message.into(),
        })
    }
}

fn validate(process: ProcessIdentity) -> Result<()> {
    monitor::validate(process).map_err(|_| {
        PluginError::new(
            ErrorCode::StaleProcess,
            "process exited, cannot be read, or its PID was reused",
        )
    })
}

fn same_namespace(pid: i32) -> Result<()> {
    let ours = fs::read_link("/proc/self/ns/pid").map_err(|_| {
        PluginError::new(
            ErrorCode::PermissionDenied,
            "cannot inspect the current PID namespace",
        )
    })?;
    let theirs = fs::read_link(format!("/proc/{pid}/ns/pid")).map_err(|_| {
        PluginError::new(
            ErrorCode::Unsupported,
            "Firefox's PID is not visible; sandbox PID translation is not supported",
        )
    })?;
    if ours != theirs {
        return Err(PluginError::new(
            ErrorCode::Unsupported,
            "Firefox uses a different PID namespace; sandbox PID translation is not supported",
        ));
    }
    Ok(())
}

fn owner(process: ProcessIdentity, browser_pid: i32) -> Result<ProcessIdentity> {
    validate(process)?;
    same_namespace(browser_pid)?;
    let mut current = process.pid;
    for _ in 0..128 {
        if current == browser_pid {
            return monitor::identity(browser_pid)
                .map_err(|_| PluginError::new(ErrorCode::StaleProcess, "browser exited"));
        }
        if current <= 1 {
            break;
        }
        let stat = fs::read_to_string(format!("/proc/{current}/stat")).map_err(|_| {
            PluginError::new(ErrorCode::StaleProcess, "cannot inspect process ancestry")
        })?;
        current = monitor::parse_stat(&stat, 4096)
            .map_err(|_| PluginError::new(ErrorCode::Unsupported, "invalid Linux process stat"))?
            .parent;
    }
    Err(PluginError::new(
        ErrorCode::Unsupported,
        "selected process does not belong to this Firefox instance",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_endpoints_is_a_useful_diagnosis() {
        let plugin = Firefox::default();
        assert!(matches!(
            plugin.handle(Operation::Diagnose).unwrap(),
            Reply::Diagnosis {
                available: false,
                ..
            }
        ));
        assert!(matches!(
            plugin.handle(Operation::Describe).unwrap(),
            Reply::Description { .. }
        ));
    }
    #[test]
    fn validates_ancestry_and_process_lifetime() {
        let id = monitor::identity(std::process::id() as i32).unwrap();
        assert_eq!(owner(id, id.pid).unwrap(), id);
        let mut stale = id;
        stale.start_ticks += 1;
        assert!(owner(stale, id.pid).is_err());
        assert!(same_namespace(i32::MAX).is_err());
    }
    #[test]
    fn rejects_stale_or_unconfigured_activation_without_connecting() {
        let id = monitor::identity(std::process::id() as i32).unwrap();
        let token = serde_json::to_string(&TargetToken {
            browser: id,
            endpoint: "127.0.0.1:2828".parse().unwrap(),
            target: "x".into(),
        })
        .unwrap();
        assert_eq!(
            Firefox::default().activate(id, &token).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
}
