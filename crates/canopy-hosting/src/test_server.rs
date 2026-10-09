//! A scripted HTTP listener on 127.0.0.1 for tests of how the client reports network failures:
//! it can answer, hang up without a response, stall, or redirect.

#![cfg(test)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What the server does with one request.
pub(crate) enum Reply {
    /// A complete response: status, extra header lines (each ending in CRLF) and body.
    Status(u16, String, String),
    /// Read the request, then close the connection without answering.
    HangUp,
    /// Read the request, then say nothing until the client gives up.
    Stall,
}

/// A running listener; its thread ends with the process.
pub(crate) struct Scripted {
    /// `http://127.0.0.1:<port>`.
    pub base: String,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Scripted {
    /// Starts a listener that answers every request target with `reply(target)`.
    pub fn start(reply: impl Fn(&str) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        let seen: Arc<Mutex<Vec<String>>> = Arc::default();
        let (log, reply) = (Arc::clone(&seen), Arc::new(reply));
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (log, reply) = (Arc::clone(&log), Arc::clone(&reply));
                std::thread::spawn(move || serve(stream, &log, &*reply));
            }
        });
        Self { base, seen }
    }

    /// The request targets received so far.
    pub fn seen(&self) -> Vec<String> {
        self.seen.lock().expect("seen").clone()
    }
}

fn serve(stream: TcpStream, log: &Mutex<Vec<String>>, reply: &dyn Fn(&str) -> Reply) {
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(clone);
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let target = line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 || header.trim_end().is_empty() {
            break;
        }
    }
    log.lock().expect("seen").push(target.clone());
    let mut stream = stream;
    match reply(&target) {
        Reply::Status(status, headers, body) => {
            let head = format!(
                "HTTP/1.1 {status} Scripted\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body.as_bytes());
        }
        Reply::HangUp => {}
        Reply::Stall => {
            // Returns when the client closes its end.
            let mut sink = [0u8; 64];
            while matches!(reader.read(&mut sink), Ok(n) if n > 0) {}
        }
    }
}

/// An address on 127.0.0.1 that refuses connections and that no server of this process can be
/// handed: a port below every operating system's ephemeral range that nothing listens on.
pub(crate) fn refused() -> SocketAddr {
    (1..=16u16)
        .map(|port| SocketAddr::from(([127, 0, 0, 1], port)))
        .find(|address| {
            TcpStream::connect_timeout(address, Duration::from_secs(2))
                .is_err_and(|e| e.kind() == std::io::ErrorKind::ConnectionRefused)
        })
        .expect("a port below 17 on 127.0.0.1 that refuses connections")
}
