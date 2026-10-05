//! Messages between two PCs, each a kind and a body: length-prefixed over
//! a non-blocking TCP connection, one to a WebSocket message (online), or
//! handed straight across between the two ends of a pair in this process
//! (for tests). For testing play over a slow network, a connection can hold
//! back what it sends and receives for a while (`Lag`).

use crate::ws::Stream;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::time::{Duration, Instant};
use tungstenite::WebSocket;

/// Larger messages mean a broken or hostile peer.
pub(crate) const MAX_MESSAGE: usize = 1 << 20;
/// A peer that falls this far behind is dropped.
pub(crate) const MAX_BACKLOG: usize = 8 << 20;
/// Messages on their way between the two ends of a pair, as a socket's
/// buffer would hold them; more wait at the sending end.
const PAIR_BUFFER: usize = 64;

pub(crate) type Message = (u8, Vec<u8>);

/// A connection to another PC.
pub struct Connection {
    link: Link,
    closed: Option<String>,
    /// Messages put back after they were received, to be received again.
    back: Vec<Message>,
    /// When we last sent something, and last heard something.
    sent: Instant,
    heard: Instant,
    /// Messages held back each way, to make the network seem slower.
    lag: Option<Box<Lagging>>,
}

/// How much slower to make a connection seem (for testing).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Lag {
    /// Added to the round trip: half on the way out, half on the way in.
    pub round_trip: Duration,
    /// Up to this much more each way, at random (messages still arrive in
    /// the order sent).
    pub jitter: Duration,
    /// The share of messages lost on the way (0-1). As over TCP, a lost
    /// one comes a round trip later, and holds up those behind it.
    pub loss: f32,
}

impl Lag {
    /// The lag the test hooks ask for: H2_NET_LAG=<round trip in ms>,
    /// H2_NET_JITTER=<ms> and H2_NET_LOSS=<percent>. None if none is set.
    pub fn from_env() -> Option<Lag> {
        let var = |name| std::env::var(name).ok().and_then(|v| v.parse::<f32>().ok());
        let (round_trip, jitter, loss) =
            (var("H2_NET_LAG"), var("H2_NET_JITTER"), var("H2_NET_LOSS"));
        if round_trip.is_none() && jitter.is_none() && loss.is_none() {
            return None;
        }
        let ms = |v: Option<f32>| Duration::from_secs_f32(v.unwrap_or(0.0).max(0.0) / 1000.0);
        Some(Lag {
            round_trip: ms(round_trip),
            jitter: ms(jitter),
            loss: (loss.unwrap_or(0.0) / 100.0).clamp(0.0, 1.0),
        })
    }
}

/// Messages held back on a lagged connection, each with when it's due.
type Held = VecDeque<(Instant, Message)>;

/// What a lagged connection holds back each way.
struct Lagging {
    lag: Lag,
    out: Held,
    inbound: Held,
    random: u32,
}

impl Lagging {
    fn new(lag: Lag) -> Lagging {
        let seed = crate::session_id();
        Lagging {
            lag,
            out: Held::new(),
            inbound: Held::new(),
            random: (seed ^ seed >> 32) as u32 | 1,
        }
    }

    /// A random number in 0..1.
    fn random(&mut self) -> f32 {
        let mut x = self.random;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.random = x;
        (x >> 8) as f32 / (1 << 24) as f32
    }

    /// Hold a message going out (or coming in) until it's due: half the
    /// round trip and some jitter from now, a round trip more if lost, and
    /// never before the one ahead of it.
    fn hold(&mut self, outgoing: bool, m: Message) {
        let lag = self.lag;
        let mut wait = lag.round_trip / 2 + lag.jitter.mul_f32(self.random());
        if self.random() < lag.loss {
            wait += lag.round_trip;
        }
        let held = if outgoing {
            &mut self.out
        } else {
            &mut self.inbound
        };
        let due = Instant::now() + wait;
        let due = held.back().map_or(due, |m| m.0.max(due));
        held.push_back((due, m));
    }
}

/// The messages in `held` that have waited long enough.
fn due(held: &mut Held) -> Vec<Message> {
    let now = Instant::now();
    let n = held.iter().take_while(|m| m.0 <= now).count();
    held.drain(..n).map(|m| m.1).collect()
}

enum Link {
    Tcp {
        stream: TcpStream,
        inbox: Vec<u8>,
        outbox: Vec<u8>,
    },
    Memory {
        tx: SyncSender<Message>,
        rx: Receiver<Message>,
        /// Messages the other end has no room for yet, and their size.
        outbox: VecDeque<Message>,
        queued: usize,
    },
    /// Each message a WebSocket message: the kind, then the body.
    Ws {
        socket: Box<WebSocket<Stream>>,
        /// Messages not given to tungstenite yet, and their size. It gets
        /// them once it has written out all it had, so what it holds is
        /// never more than it was last given.
        outbox: VecDeque<Vec<u8>>,
        queued: usize,
        /// The size of what it was last given while some may be left, and
        /// of its answers to pings since it last wrote out all it had.
        handed: usize,
    },
}

impl Connection {
    pub fn tcp(stream: TcpStream) -> io::Result<Connection> {
        stream.set_nonblocking(true)?;
        stream.set_nodelay(true)?;
        Ok(Connection::over(Link::Tcp {
            stream,
            inbox: Vec::new(),
            outbox: Vec::new(),
        }))
    }

    /// Two ends connected to each other in this process: a host and a PC
    /// joining it, say, in a test.
    pub fn pair() -> (Connection, Connection) {
        let (a, b) = (
            mpsc::sync_channel(PAIR_BUFFER),
            mpsc::sync_channel(PAIR_BUFFER),
        );
        let end = |tx, rx| {
            Connection::over(Link::Memory {
                tx,
                rx,
                outbox: VecDeque::new(),
                queued: 0,
            })
        };
        (end(a.0, b.1), end(b.0, a.1))
    }

    /// A WebSocket, its handshake done (see `dial` and `accept`).
    pub(crate) fn ws(mut socket: WebSocket<Stream>) -> io::Result<Connection> {
        socket.get_mut().set_nonblocking()?;
        Ok(Connection::over(Link::Ws {
            socket: Box::new(socket),
            outbox: VecDeque::new(),
            queued: 0,
            handed: 0,
        }))
    }

    fn over(link: Link) -> Connection {
        Connection {
            link,
            closed: None,
            back: Vec::new(),
            sent: Instant::now(),
            heard: Instant::now(),
            lag: None,
        }
    }

    /// Hold back what goes either way by `lag` from now on (for testing).
    pub fn set_lag(&mut self, lag: Lag) {
        self.lag = Some(Box::new(Lagging::new(lag)));
    }

    /// The TCP connection it's over, to wait for something to come on
    /// (none between the two ends of a pair).
    pub fn stream(&self) -> Option<&TcpStream> {
        match &self.link {
            Link::Tcp { stream, .. } => Some(stream),
            Link::Memory { .. } => None,
            Link::Ws { socket, .. } => Some(socket.get_ref().tcp()),
        }
    }

    /// How long since something last arrived.
    pub fn since_heard(&self) -> Duration {
        self.heard.elapsed()
    }

    /// How long since something was last sent.
    pub fn since_sent(&self) -> Duration {
        self.sent.elapsed()
    }

    /// After `quiet` with nothing sent, send an empty `kind` message so the
    /// other end knows we're still here.
    pub fn keep_alive(&mut self, kind: u8, quiet: Duration) {
        if self.sent.elapsed() >= quiet {
            self.send(kind, &[]);
        }
    }

    pub fn send(&mut self, kind: u8, body: &[u8]) {
        self.sent = Instant::now();
        match &mut self.lag {
            Some(lag) => lag.hold(true, (kind, body.to_vec())),
            None => self.send_now(kind, body),
        }
    }

    fn send_now(&mut self, kind: u8, body: &[u8]) {
        match &mut self.link {
            Link::Tcp { outbox, .. } => {
                outbox.extend_from_slice(&(body.len() as u32 + 1).to_le_bytes());
                outbox.push(kind);
                outbox.extend_from_slice(body);
            }
            Link::Memory { outbox, queued, .. } => {
                *queued += body.len() + 1;
                outbox.push_back((kind, body.to_vec()));
            }
            Link::Ws { outbox, queued, .. } => {
                let mut m = Vec::with_capacity(body.len() + 1);
                m.push(kind);
                m.extend_from_slice(body);
                *queued += m.len();
                outbox.push_back(m);
            }
        }
    }

    /// Bytes sent that the network (or the other end) hasn't taken yet.
    pub fn backlog(&self) -> usize {
        match &self.link {
            Link::Tcp { outbox, .. } => outbox.len(),
            Link::Memory { queued, .. } => *queued,
            Link::Ws { queued, handed, .. } => queued + handed,
        }
    }

    /// Write out as much as the network takes now. Err once the connection
    /// is gone.
    pub fn flush(&mut self) -> Result<(), String> {
        if let Some(why) = &self.closed {
            return Err(why.clone());
        }
        let held = self.lag.as_mut().map(|lag| due(&mut lag.out));
        for (kind, body) in held.unwrap_or_default() {
            self.send_now(kind, &body);
        }
        let gone = match &mut self.link {
            Link::Tcp { stream, outbox, .. } => write_out(stream, outbox),
            Link::Memory {
                tx, outbox, queued, ..
            } => hand_over(tx, outbox, queued),
            Link::Ws {
                socket,
                outbox,
                queued,
                handed,
            } => write_ws(socket, outbox, queued, handed),
        };
        if let Some(why) = gone {
            return self.close(&why);
        }
        if self.backlog() > MAX_BACKLOG {
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
    pub fn receive(&mut self) -> Result<Vec<Message>, String> {
        let mut out = std::mem::take(&mut self.back);
        if let Some(why) = &self.closed {
            return if out.is_empty() {
                Err(why.clone())
            } else {
                Ok(out)
            };
        }
        let mut arrived = Vec::new();
        let ended = match &mut self.link {
            Link::Tcp { stream, inbox, .. } => {
                let ended = read_in(stream, inbox);
                if !split(inbox, &mut arrived) {
                    return self.close("bad message");
                }
                ended
            }
            Link::Memory { rx, .. } => loop {
                match rx.try_recv() {
                    Ok(m) => arrived.push(m),
                    Err(TryRecvError::Empty) => break None,
                    Err(TryRecvError::Disconnected) => break Some("connection closed".into()),
                }
            },
            Link::Ws { socket, handed, .. } => read_ws(socket, handed, &mut arrived),
        };
        match &mut self.lag {
            // What arrived waits its turn (and what's held comes at once
            // when the connection goes).
            Some(lag) => {
                for m in arrived {
                    lag.hold(false, m);
                }
                match ended {
                    Some(_) => out.extend(lag.inbound.drain(..).map(|m| m.1)),
                    None => out.extend(due(&mut lag.inbound)),
                }
            }
            None => out.append(&mut arrived),
        }
        if !out.is_empty() {
            self.heard = Instant::now();
        }
        if let Some(why) = ended {
            self.closed = Some(why.clone());
            if out.is_empty() {
                return Err(why);
            }
        }
        Ok(out)
    }

    /// Give back `messages`, the last of those `receive` returned: the next
    /// `receive` returns them first. (A relay leg reads the server's word
    /// that it's linked, and leaves what came after it for the game.)
    pub fn put_back(&mut self, mut messages: Vec<Message>) {
        messages.append(&mut self.back);
        self.back = messages;
    }
}

/// Write as much of `outbox` as the network takes now. Why the connection
/// is gone, if it is.
fn write_out(stream: &mut TcpStream, outbox: &mut Vec<u8>) -> Option<String> {
    while !outbox.is_empty() {
        match stream.write(outbox) {
            Ok(0) => return Some("connection closed".into()),
            Ok(n) => {
                outbox.drain(..n);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Some(io_gone(e)),
        }
    }
    None
}

/// Read what has arrived into `inbox`. Why the connection is gone, if it
/// is.
fn read_in(stream: &mut TcpStream, inbox: &mut Vec<u8>) -> Option<String> {
    let mut buf = [0u8; 64 * 1024];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => return Some("connection closed".into()),
            Ok(n) => inbox.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return None,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Some(io_gone(e)),
        }
    }
}

/// Take the complete messages off the front of `inbox`. False if it holds
/// something that isn't a message.
fn split(inbox: &mut Vec<u8>, out: &mut Vec<Message>) -> bool {
    while inbox.len() >= 4 {
        let len = u32::from_le_bytes(inbox[..4].try_into().unwrap()) as usize;
        if len == 0 || len > MAX_MESSAGE {
            return false;
        }
        if inbox.len() < 4 + len {
            break;
        }
        out.push((inbox[4], inbox[5..4 + len].to_vec()));
        inbox.drain(..4 + len);
    }
    true
}

/// Pass the other end of a pair what it has room for. Why the connection
/// is gone, if it is.
fn hand_over(
    tx: &SyncSender<Message>,
    outbox: &mut VecDeque<Message>,
    queued: &mut usize,
) -> Option<String> {
    while let Some(m) = outbox.pop_front() {
        let size = m.1.len() + 1;
        match tx.try_send(m) {
            Ok(()) => *queued -= size,
            Err(TrySendError::Full(m)) => {
                outbox.push_front(m);
                break;
            }
            Err(TrySendError::Disconnected(_)) => return Some("connection closed".into()),
        }
    }
    None
}

/// Give tungstenite the messages waiting once it has written out all it
/// had, and write out as much as the network takes now. Why the connection
/// is gone, if it is.
fn write_ws(
    socket: &mut WebSocket<Stream>,
    outbox: &mut VecDeque<Vec<u8>>,
    queued: &mut usize,
    handed: &mut usize,
) -> Option<String> {
    loop {
        match socket.flush() {
            Ok(()) => *handed = 0,
            Err(e) => return ws_gone(e),
        }
        if outbox.is_empty() {
            return None;
        }
        *handed = std::mem::take(queued);
        for m in outbox.drain(..) {
            // What the network doesn't take now, tungstenite keeps.
            let m = tungstenite::Message::Binary(m.into());
            if let Some(why) = socket.write(m).err().and_then(ws_gone) {
                return Some(why);
            }
        }
    }
}

/// Read the messages that have arrived. Why the connection is gone, if it
/// is.
fn read_ws(
    socket: &mut WebSocket<Stream>,
    handed: &mut usize,
    out: &mut Vec<Message>,
) -> Option<String> {
    use tungstenite::Message as Ws;
    loop {
        match socket.read() {
            Ok(Ws::Binary(m)) if !m.is_empty() => out.push((m[0], m[1..].to_vec())),
            // tungstenite answers pings itself, after what it has to write
            // out: so something that pings and never reads soon has us too
            // far behind.
            Ok(Ws::Ping(m)) => {
                // The answer, and its header.
                *handed += m.len() + 2;
                if *handed > MAX_BACKLOG {
                    return Some("connection too slow".into());
                }
            }
            Ok(Ws::Pong(_)) => {}
            Ok(Ws::Close(_)) => return Some("connection closed".into()),
            Ok(_) => return Some("bad message".into()),
            Err(e) => return ws_gone(e),
        }
    }
}

/// Why a WebSocket is gone, from what went wrong with it. None if nothing
/// did: it just can't go on until the network has more for it, or takes
/// more.
fn ws_gone(e: tungstenite::Error) -> Option<String> {
    use io::ErrorKind::{Interrupted, WouldBlock};
    use tungstenite::error::{Error, ProtocolError};
    match e {
        Error::Io(e) if matches!(e.kind(), WouldBlock | Interrupted) => None,
        Error::Io(e) => Some(io_gone(e)),
        Error::ConnectionClosed
        | Error::AlreadyClosed
        | Error::Protocol(ProtocolError::ResetWithoutClosingHandshake) => {
            Some("connection closed".into())
        }
        Error::Capacity(_) => Some("bad message".into()),
        // More to write out than it may hold.
        Error::WriteBufferFull(_) => Some("connection too slow".into()),
        e => Some(e.to_string()),
    }
}

/// Why a connection is gone, from what went wrong reading or writing it.
fn io_gone(e: io::Error) -> String {
    use io::ErrorKind::{BrokenPipe, ConnectionAborted, ConnectionReset, UnexpectedEof};
    match e.kind() {
        // However the other end went: reset, say, or over TLS without
        // saying goodbye.
        UnexpectedEof | ConnectionReset | ConnectionAborted | BrokenPipe => {
            "connection closed".into()
        }
        _ => e.to_string(),
    }
}
