//! Version 1 of the executable plugin interface. stdout is JSON Lines only.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};

pub const VERSION: u32 = 1;
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct ProcessIdentity {
    pub pid: i32,
    /// Linux /proc/PID/stat starttime, in clock ticks since boot.
    pub start_ticks: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Request {
    pub version: u32,
    pub id: u64,
    #[serde(flatten)]
    pub operation: Operation,
}

impl Request {
    pub fn new(id: u64, operation: Operation) -> Self {
        Self {
            version: VERSION,
            id,
            operation,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Operation {
    Describe,
    Diagnose,
    Resolve {
        process: ProcessIdentity,
    },
    Activate {
        process: ProcessIdentity,
        target_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Response {
    pub version: u32,
    pub id: u64,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", content = "data", rename_all = "snake_case")]
pub enum Outcome {
    Ok(Reply),
    Error(PluginError),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Reply {
    Description {
        name: String,
        version: String,
        capabilities: Vec<String>,
    },
    Diagnosis {
        available: bool,
        message: String,
    },
    Resolution(Resolution),
    Activation(Activation),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Resolution {
    pub process: ProcessIdentity,
    pub application: String,
    pub summary: String,
    pub targets: Vec<Target>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Target {
    /// Opaque to the monitor. Must be revalidated by the plugin on activation.
    pub id: String,
    pub title: String,
    pub url: Option<String>,
    pub window: String,
    pub relationship: Relationship,
    pub can_activate: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Relationship {
    MainPage,
    EmbeddedFrame,
    Application,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Activation {
    pub selected: bool,
    pub focus: FocusResult,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FocusResult {
    Confirmed,
    Requested,
    Denied,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PluginError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    IncompatibleVersion,
    InvalidRequest,
    Unavailable,
    PermissionDenied,
    StaleProcess,
    StaleTarget,
    Unsupported,
    Internal,
}

impl PluginError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}
impl std::error::Error for PluginError {}

/// Serve bounded JSON Lines. Invalid envelopes receive id 0 and do not end the session.
pub fn serve(
    mut input: impl BufRead,
    mut output: impl Write,
    mut handle: impl FnMut(Operation) -> Result<Reply, PluginError>,
) -> std::io::Result<()> {
    loop {
        let mut line = Vec::new();
        let count = std::io::Read::take(&mut input, (MAX_MESSAGE_BYTES + 1) as u64)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            return Ok(());
        }
        if line.len() > MAX_MESSAGE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "request exceeds message limit",
            ));
        }
        let (id, result) = match serde_json::from_slice::<Request>(&line) {
            Ok(request) if request.version == VERSION => (request.id, handle(request.operation)),
            Ok(request) => (
                request.id,
                Err(PluginError::new(
                    ErrorCode::IncompatibleVersion,
                    "supported protocol version is 1",
                )),
            ),
            Err(_) => (
                0,
                Err(PluginError::new(
                    ErrorCode::InvalidRequest,
                    "invalid request envelope",
                )),
            ),
        };
        let response = Response {
            version: VERSION,
            id,
            outcome: match result {
                Ok(reply) => Outcome::Ok(reply),
                Err(error) => Outcome::Error(error),
            },
        };
        let bytes = serde_json::to_vec(&response)?;
        if bytes.len() >= MAX_MESSAGE_BYTES {
            return Err(std::io::Error::other("response exceeds message limit"));
        }
        output.write_all(&bytes)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn serves_multiple_lines_and_recovers_from_invalid_input() {
        let input = b"oops\n{\"version\":9,\"id\":12,\"method\":\"describe\"}\n{\"version\":1,\"id\":13,\"method\":\"diagnose\"}\n";
        let mut output = Vec::new();
        serve(Cursor::new(input), &mut output, |_| {
            Ok(Reply::Diagnosis {
                available: true,
                message: "ready".into(),
            })
        })
        .unwrap();
        let replies: Vec<Response> = output
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_slice(l).unwrap())
            .collect();
        assert_eq!(replies.len(), 3);
        assert!(matches!(
            replies[0].outcome,
            Outcome::Error(PluginError {
                code: ErrorCode::InvalidRequest,
                ..
            })
        ));
        assert_eq!(replies[1].id, 12);
        assert!(matches!(
            replies[1].outcome,
            Outcome::Error(PluginError {
                code: ErrorCode::IncompatibleVersion,
                ..
            })
        ));
        assert_eq!(replies[2].id, 13);
        assert!(matches!(
            replies[2].outcome,
            Outcome::Ok(Reply::Diagnosis {
                available: true,
                ..
            })
        ));
    }

    #[test]
    fn rejects_unbounded_input_before_dispatch() {
        let input = vec![b'x'; MAX_MESSAGE_BYTES + 2];
        assert!(
            serve(Cursor::new(input), Vec::new(), |_| panic!(
                "must not dispatch"
            ))
            .is_err()
        );
    }

    #[test]
    fn identity_round_trip() {
        let req = Request::new(
            4,
            Operation::Resolve {
                process: ProcessIdentity {
                    pid: 12,
                    start_ticks: 42,
                },
            },
        );
        let value = serde_json::to_value(&req).unwrap();
        assert_eq!(value["params"]["process"]["start_ticks"], 42);
        assert_eq!(value["method"], "resolve");
        assert!(serde_json::from_value::<Request>(value).is_ok());
    }
}
