use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};
use top_dog_protocol::{ErrorCode, MAX_MESSAGE_BYTES, PluginError};

pub type Result<T> = std::result::Result<T, PluginError>;

pub fn endpoint(text: &str) -> Result<SocketAddr> {
    let address: SocketAddr = text.parse().map_err(|_| {
        PluginError::new(
            ErrorCode::InvalidRequest,
            "endpoint must be a numeric loopback IP:port",
        )
    })?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err(PluginError::new(
            ErrorCode::InvalidRequest,
            "only loopback endpoints with a nonzero port are supported",
        ));
    }
    Ok(address)
}

pub struct Client<S: Read + Write> {
    stream: S,
    next_id: u64,
    session: bool,
}

impl Client<TcpStream> {
    pub fn connect(address: SocketAddr) -> Result<Self> {
        let stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).map_err(|_| PluginError::new(ErrorCode::Unavailable, "cannot connect to Firefox; start it with --marionette and --remote-allow-system-access on the configured loopback port"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(transport_error)?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(transport_error)?;
        Self::handshake(stream)
    }
}

impl<S: Read + Write> Client<S> {
    fn handshake(stream: S) -> Result<Self> {
        let mut client = Self {
            stream,
            next_id: 0,
            session: false,
        };
        let greeting = read_frame(&mut client.stream)?;
        if greeting["marionetteProtocol"] != 3 || greeting["applicationType"] != "gecko" {
            return Err(PluginError::new(
                ErrorCode::Unsupported,
                "unsupported Marionette handshake; expected Gecko protocol 3",
            ));
        }
        client.command(
            "WebDriver:NewSession",
            json!({"capabilities":{"alwaysMatch":{}}}),
        )?;
        client.session = true;
        client
            .command("Marionette:SetContext", json!({"value":"chrome"}))
            .map_err(|_| {
                PluginError::new(
                    ErrorCode::PermissionDenied,
                    "Firefox system access unavailable; restart with --remote-allow-system-access",
                )
            })?;
        Ok(client)
    }

    pub fn command(&mut self, name: &str, params: Value) -> Result<Value> {
        self.next_id += 1;
        write_frame(&mut self.stream, &json!([0, self.next_id, name, params]))?;
        let response = read_frame(&mut self.stream)?;
        if !response.is_array()
            || response.as_array().map(Vec::len) != Some(4)
            || response[0] != 1
            || response[1] != self.next_id
        {
            return Err(PluginError::new(
                ErrorCode::Internal,
                "invalid Marionette response envelope",
            ));
        }
        if !response[2].is_null() {
            // Firefox errors can include script source and URLs. Keep diagnostics generic.
            return Err(PluginError::new(
                ErrorCode::Unsupported,
                format!(
                    "Firefox rejected {name}; browser internals may be incompatible or the browser may be busy"
                ),
            ));
        }
        Ok(response[3].clone())
    }

    pub fn execute(&mut self, script: &str, options: Value) -> Result<Value> {
        let value = self.command("WebDriver:ExecuteScript", json!({"script":script,"args":[options],"newSandbox":true,"sandbox":"top-dog","filename":"top-dog-bridge","line":1}))?;
        value.get("value").cloned().ok_or_else(|| {
            PluginError::new(
                ErrorCode::Unsupported,
                "Firefox returned an incompatible script result",
            )
        })
    }
}

impl<S: Read + Write> Drop for Client<S> {
    fn drop(&mut self) {
        if self.session {
            // Marionette DeleteSession detaches automation; it does not quit Firefox.
            let _ = self.command("WebDriver:DeleteSession", json!({}));
            self.session = false;
        }
    }
}

fn transport_error(_: std::io::Error) -> PluginError {
    PluginError::new(
        ErrorCode::Unavailable,
        "Firefox automation connection closed or timed out",
    )
}

fn read_frame(stream: &mut impl Read) -> Result<Value> {
    let mut length = 0usize;
    for digits in 0..=7 {
        let mut byte = [0];
        stream.read_exact(&mut byte).map_err(transport_error)?;
        if byte[0] == b':' {
            if digits == 0 || length == 0 || length > MAX_MESSAGE_BYTES {
                break;
            }
            let mut data = vec![0; length];
            stream.read_exact(&mut data).map_err(transport_error)?;
            return serde_json::from_slice(&data).map_err(|_| {
                PluginError::new(ErrorCode::Internal, "invalid JSON in Marionette frame")
            });
        }
        if !byte[0].is_ascii_digit() {
            break;
        }
        length = length * 10 + (byte[0] - b'0') as usize;
    }
    Err(PluginError::new(
        ErrorCode::Internal,
        "invalid or oversized Marionette frame length",
    ))
}

fn write_frame(stream: &mut impl Write, value: &Value) -> Result<()> {
    let data = serde_json::to_vec(value)
        .map_err(|_| PluginError::new(ErrorCode::Internal, "cannot serialize Marionette frame"))?;
    if data.len() > MAX_MESSAGE_BYTES {
        return Err(PluginError::new(
            ErrorCode::Internal,
            "Marionette frame exceeds message limit",
        ));
    }
    write!(stream, "{}:", data.len()).map_err(transport_error)?;
    stream.write_all(&data).map_err(transport_error)?;
    stream.flush().map_err(transport_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn endpoint_rejects_remote_hosts_and_dns_names() {
        for bad in [
            "0.0.0.0:2828",
            "192.168.1.2:2828",
            "localhost:2828",
            "127.0.0.1:0",
        ] {
            assert!(endpoint(bad).is_err());
        }
        assert!(endpoint("127.0.0.1:2828").is_ok());
        assert!(endpoint("[::1]:2828").is_ok());
    }
    #[test]
    fn framing_uses_utf8_byte_lengths_and_bounds_input() {
        let mut bytes = Vec::new();
        let value = json!({"title":"日本語"});
        write_frame(&mut bytes, &value).unwrap();
        assert_eq!(read_frame(&mut Cursor::new(bytes)).unwrap(), value);
        for bad in ["9999999:", ":", "x:", "2:{", "2:xx", "0:"] {
            assert!(read_frame(&mut Cursor::new(bad)).is_err(), "{bad}");
        }
    }
    struct Duplex {
        input: Cursor<Vec<u8>>,
        output: Vec<u8>,
    }
    impl Read for Duplex {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(b)
        }
    }
    impl Write for Duplex {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.output.write(b)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn handshake_script_response_and_detach_follow_protocol() {
        let mut input = Vec::new();
        for value in [
            json!({"marionetteProtocol":3,"applicationType":"gecko"}),
            json!([1, 1, null, {}]),
            json!([1, 2, null, null]),
            json!([1,3,null,{"value":{"applicationPid":42}}]),
            json!([1, 4, null, null]),
        ] {
            write_frame(&mut input, &value).unwrap();
        }
        let mut c = Client::handshake(Duplex {
            input: Cursor::new(input),
            output: Vec::new(),
        })
        .unwrap();
        assert_eq!(
            c.execute("return 42", json!({})).unwrap()["applicationPid"],
            42
        );
        let mut sent = Cursor::new(&c.stream.output);
        assert_eq!(read_frame(&mut sent).unwrap()[2], "WebDriver:NewSession");
        assert_eq!(read_frame(&mut sent).unwrap()[2], "Marionette:SetContext");
        assert_eq!(read_frame(&mut sent).unwrap()[2], "WebDriver:ExecuteScript");
    }
    #[test]
    fn rejects_wrong_response_id() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &json!([1, 99, null, {}])).unwrap();
        let mut c = Client {
            stream: Duplex {
                input: Cursor::new(bytes),
                output: Vec::new(),
            },
            next_id: 0,
            session: false,
        };
        assert!(c.command("test", json!({})).is_err());
    }
}
