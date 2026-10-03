//! Length-prefixed messages over a non-blocking TCP connection.

use std::io::{self, Read, Write};
use std::net::TcpStream;

/// Larger messages mean a broken or hostile peer.
const MAX_MESSAGE: usize = 1 << 20;
/// A peer that falls this far behind is dropped.
const MAX_BACKLOG: usize = 8 << 20;

pub struct Connection {
    stream: TcpStream,
    inbox: Vec<u8>,
    outbox: Vec<u8>,
    closed: Option<String>,
}

impl Connection {
    pub fn new(stream: TcpStream) -> io::Result<Connection> {
        stream.set_nonblocking(true)?;
        stream.set_nodelay(true)?;
        Ok(Connection {
            stream,
            inbox: Vec::new(),
            outbox: Vec::new(),
            closed: None,
        })
    }

    pub fn send(&mut self, kind: u8, body: &[u8]) {
        self.outbox
            .extend_from_slice(&(body.len() as u32 + 1).to_le_bytes());
        self.outbox.push(kind);
        self.outbox.extend_from_slice(body);
    }

    /// Write out as much as the network takes now. Err once the connection
    /// is gone.
    pub fn flush(&mut self) -> Result<(), String> {
        if let Some(why) = &self.closed {
            return Err(why.clone());
        }
        while !self.outbox.is_empty() {
            match self.stream.write(&self.outbox) {
                Ok(0) => return self.close("connection closed"),
                Ok(n) => {
                    self.outbox.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return self.close(&e.to_string()),
            }
        }
        if self.outbox.len() > MAX_BACKLOG {
            return self.close("connection too slow");
        }
        Ok(())
    }

    fn close<T>(&mut self, why: &str) -> Result<T, String> {
        self.closed = Some(why.to_string());
        Err(why.to_string())
    }

    /// Complete messages received so far, as (kind, body). Messages that
    /// arrived before the connection closed are still returned; the error
    /// comes with the next call.
    pub fn receive(&mut self) -> Result<Vec<(u8, Vec<u8>)>, String> {
        if let Some(why) = &self.closed {
            return Err(why.clone());
        }
        let mut buf = [0u8; 64 * 1024];
        let mut ended = None;
        loop {
            match self.stream.read(&mut buf) {
                Ok(0) => {
                    ended = Some("connection closed".to_string());
                    break;
                }
                Ok(n) => self.inbox.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => {
                    ended = Some(e.to_string());
                    break;
                }
            }
        }
        let mut out = Vec::new();
        while self.inbox.len() >= 4 {
            let len = u32::from_le_bytes(self.inbox[..4].try_into().unwrap()) as usize;
            if len == 0 || len > MAX_MESSAGE {
                return self.close("bad message");
            }
            if self.inbox.len() < 4 + len {
                break;
            }
            let kind = self.inbox[4];
            out.push((kind, self.inbox[5..4 + len].to_vec()));
            self.inbox.drain(..4 + len);
        }
        if let Some(why) = ended {
            self.closed = Some(why.clone());
            if out.is_empty() {
                return Err(why);
            }
        }
        Ok(out)
    }
}
