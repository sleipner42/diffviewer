use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};
use std::thread;

use anyhow::{Context, Result};

use crate::application::repository::RangeSpec;

pub enum Launch {
    Primary(UnixListener),
    Forwarded,
}

/// What a `dw` invocation asks the window to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRequest {
    pub dir: PathBuf,
    pub range: Option<RangeSpec>,
}

pub fn claim_or_forward(request: &OpenRequest) -> Result<Launch> {
    let socket = socket_path();
    if let Ok(mut stream) = UnixStream::connect(&socket) {
        let message = encode(request);
        stream
            .write_all(message.as_bytes())
            .context("sending path to the running diffviewer")?;
        let mut reply = String::new();
        let _ = BufReader::new(stream).read_line(&mut reply);
        return Ok(Launch::Forwarded);
    }
    if socket.exists() {
        let _ = std::fs::remove_file(&socket);
    }
    let listener = UnixListener::bind(&socket).with_context(|| format!("binding {}", socket.display()))?;
    Ok(Launch::Primary(listener))
}

pub fn spawn_listener(listener: UnixListener, wake: Box<dyn Fn() + Send>) -> Receiver<OpenRequest> {
    let (tx, rx) = channel();
    thread::spawn(move || {
        for connection in listener.incoming() {
            let Ok(mut stream) = connection else {
                continue;
            };
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_err() {
                continue;
            }
            if let Some(request) = decode(line.trim_end()) {
                if tx.send(request).is_err() {
                    return;
                }
                wake();
            }
            let _ = stream.write_all(REPLY);
        }
    });
    rx
}

fn encode(request: &OpenRequest) -> String {
    let mut message = format!("{OPEN_COMMAND}\t{}", request.dir.display());
    if let Some(range) = &request.range {
        message.push_str(&format!("\t{}\t{}", range.base, range.head));
    }
    message.push('\n');
    message
}

fn decode(line: &str) -> Option<OpenRequest> {
    let rest = line.strip_prefix(OPEN_COMMAND)?.strip_prefix('\t')?;
    let mut fields = rest.split('\t');
    let dir = PathBuf::from(fields.next()?);
    let range = match (fields.next(), fields.next()) {
        (Some(base), Some(head)) if !base.is_empty() && !head.is_empty() => Some(RangeSpec {
            base: base.to_string(),
            head: head.to_string(),
        }),
        _ => None,
    };
    Some(OpenRequest { dir, range })
}

fn socket_path() -> PathBuf {
    let user = std::env::var("USER").unwrap_or_else(|_| "default".to_string());
    std::env::temp_dir().join(format!("{SOCKET_PREFIX}-{user}.sock"))
}

const SOCKET_PREFIX: &str = "diffviewer";
const OPEN_COMMAND: &str = "open";
const REPLY: &[u8] = b"ok\n";

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(request: OpenRequest) {
        let encoded = encode(&request);
        assert_eq!(decode(encoded.trim_end()), Some(request));
    }

    #[test]
    fn round_trips_a_plain_directory() {
        round_trip(OpenRequest { dir: PathBuf::from("/home/x/repo"), range: None });
    }

    #[test]
    fn round_trips_a_range() {
        round_trip(OpenRequest {
            dir: PathBuf::from("/home/x/repo"),
            range: Some(RangeSpec { base: "main".into(), head: "HEAD".into() }),
        });
    }
}
