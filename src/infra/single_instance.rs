use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::thread;

use anyhow::{Context, Result};

pub enum Launch {
    Primary(UnixListener),
    Forwarded,
}

pub fn claim_or_forward(path: &Path) -> Result<Launch> {
    let socket = socket_path();
    if let Ok(mut stream) = UnixStream::connect(&socket) {
        let message = format!("{OPEN_COMMAND}\t{}\n", path.display());
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

pub fn spawn_listener(listener: UnixListener, wake: Box<dyn Fn() + Send>) -> Receiver<PathBuf> {
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
            let requested = line
                .trim_end()
                .strip_prefix(OPEN_COMMAND)
                .and_then(|rest| rest.strip_prefix('\t'))
                .map(PathBuf::from);
            if let Some(path) = requested {
                if tx.send(path).is_err() {
                    return;
                }
                wake();
            }
            let _ = stream.write_all(REPLY);
        }
    });
    rx
}

fn socket_path() -> PathBuf {
    let user = std::env::var("USER").unwrap_or_else(|_| "default".to_string());
    std::env::temp_dir().join(format!("{SOCKET_PREFIX}-{user}.sock"))
}

const SOCKET_PREFIX: &str = "diffviewer";
const OPEN_COMMAND: &str = "open";
const REPLY: &[u8] = b"ok\n";
