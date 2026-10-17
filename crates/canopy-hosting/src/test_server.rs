//! A scripted HTTP listener on 127.0.0.1 for tests of how the client reports network failures:
//! it can answer, hang up without a response, stall, or redirect.

#![cfg(test)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

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

/// An address on 127.0.0.1 that refuses connections and that no bind to port 0, in this process
/// or another, is handed: a port below every operating system's ephemeral range.
///
/// Binding a listener and dropping it does not give such an address: the port is free again. Nor
/// does holding a connected pair on it: a server another test starts on port 0 can still be
/// handed that port where the kernel is set to reuse them (`net.ipv4.ip_autobind_reuse`), and
/// Windows lets another socket bind a port a connected one uses. Only a process that names the
/// port itself can take this address, and an unprivileged one cannot on Linux.
pub(crate) struct Refused {
    pub address: SocketAddr,
}

pub(crate) fn refused() -> Refused {
    let address = (1..=16u16)
        .map(|port| SocketAddr::from(([127, 0, 0, 1], port)))
        .find(|address| {
            TcpStream::connect(address)
                .is_err_and(|e| e.kind() == std::io::ErrorKind::ConnectionRefused)
        })
        .expect("a port below 17 on 127.0.0.1 that refuses connections");
    Refused { address }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #210: the address `refused()` hands out must not be one a parallel test or process can
    /// take while the fixture is alive; a listener there answers the lookup that should be
    /// refused (`ECONNRESET` instead of `ECONNREFUSED`). Linux: an unprivileged process cannot
    /// bind below `ip_unprivileged_port_start`, and `refused()` stays below it; this fails when
    /// the process may bind low ports (root, or that sysctl lowered).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_refused_address_cannot_be_bound_while_it_is_held() {
        let closed = refused();
        let taken = TcpListener::bind(closed.address);
        assert!(
            taken.is_err(),
            "{} was bound while refused() held it",
            closed.address
        );
    }

    #[test]
    fn a_refused_address_stays_refused_and_is_never_handed_to_a_server() {
        let closed = refused();
        let connect = TcpStream::connect(closed.address).unwrap_err();
        assert_eq!(connect.kind(), std::io::ErrorKind::ConnectionRefused);
        assert!(
            closed.address.port() < 1024,
            "below every operating system's ephemeral range"
        );
        // The servers of the other tests bind to port 0 and keep what they are given.
        let servers: Vec<_> = (0..64)
            .map(|_| TcpListener::bind("127.0.0.1:0").expect("bind"))
            .collect();
        assert!(
            servers
                .iter()
                .all(|s| s.local_addr().expect("addr") != closed.address)
        );
    }
}
