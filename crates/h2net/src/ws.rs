//! WebSockets, as PCs reach the online service: a game dials the server on
//! a thread of its own (so the game goes on meanwhile), and the server
//! takes WebSockets and plain web requests (a browser's, a health check's)
//! on the same port.

use crate::conn::{Connection, MAX_BACKLOG, MAX_MESSAGE};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore};
use std::io::{self, ErrorKind, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tungstenite::handshake::server::{create_response, write_response};
use tungstenite::handshake::HandshakeError;
use tungstenite::http::Uri;
use tungstenite::protocol::{Role, WebSocket, WebSocketConfig};

/// A web request must arrive this soon.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Longer requests (than any game's or browser's) are hostile.
const MAX_REQUEST: usize = 16 << 10;

/// What a connection to a server asked for, and at what path (only the
/// path: without any query).
pub enum Request {
    /// A WebSocket, at this path (`/live`, say), and the address a proxy in
    /// front of the server (a host's) says it came from, if one does.
    /// Anyone can say that, so it's only worth believing behind a proxy.
    WebSocket(Connection, String, Option<IpAddr>),
    /// A page at this path (for a browser, or a health check): answer it
    /// with `reply`.
    Http(TcpStream, String),
}

/// What a WebSocket runs over: TCP, or TLS over TCP (to a server online).
pub(crate) enum Stream {
    Plain(Tcp),
    Tls(Box<ClientConnection>, Tcp),
}

impl Stream {
    /// Never wait from now on (the handshake done), and so never give up.
    pub(crate) fn set_nonblocking(&mut self) -> io::Result<()> {
        let (Stream::Plain(tcp) | Stream::Tls(_, tcp)) = self;
        tcp.deadline = None;
        tcp.stream.set_nonblocking(true)?;
        tcp.stream.set_nodelay(true)
    }
}

/// A TCP connection, and while it's in a handshake, when to give up on it:
/// however little comes at a time, it all has to come by then.
pub(crate) struct Tcp {
    stream: TcpStream,
    deadline: Option<Instant>,
}

impl Stream {
    /// The TCP connection under it.
    pub(crate) fn tcp(&self) -> &TcpStream {
        let (Stream::Plain(tcp) | Stream::Tls(_, tcp)) = self;
        &tcp.stream
    }
}

impl Tcp {
    /// The stream, to wait on for no longer than the time left.
    fn in_time(&self) -> io::Result<&TcpStream> {
        if let Some(deadline) = self.deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(ErrorKind::TimedOut.into());
            }
            self.stream.set_read_timeout(Some(left))?;
            self.stream.set_write_timeout(Some(left))?;
        }
        Ok(&self.stream)
    }
}

impl Read for Tcp {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.in_time()?.read(buf)
    }
}

impl Write for Tcp {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.in_time()?.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

// Not rustls's own `StreamOwned`, which can hold back what it has already
// decrypted while what it has to send waits for the network: the other end
// may be waiting to be read before it takes more.
impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let (tls, tcp) = match self {
            Stream::Plain(tcp) => return tcp.read(buf),
            Stream::Tls(tls, tcp) => (tls, tcp),
        };
        loop {
            match tls.reader().read(buf) {
                Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                done => return done,
            }
            // Nothing to hand over yet: take in more of what has arrived,
            // and answer what needs answering (in the handshake, say) as far
            // as the network takes it now. Anything wrong with writing shows
            // when flushing.
            tls.read_tls(tcp)?;
            let packets = tls.process_new_packets();
            packets.map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
            let _ = send_tls(tls, tcp);
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let (tls, tcp) = match self {
            Stream::Plain(tcp) => return tcp.write(buf),
            Stream::Tls(tls, tcp) => (tls, tcp),
        };
        // Make room first, as far as the network takes what's waiting.
        let _ = send_tls(tls, tcp);
        match tls.writer().write(buf)? {
            0 if !buf.is_empty() => Err(ErrorKind::WouldBlock.into()),
            n => Ok(n),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Plain(tcp) => tcp.flush(),
            Stream::Tls(tls, tcp) => send_tls(tls, tcp),
        }
    }
}

/// Write out what `tls` has encrypted, or has to say.
fn send_tls(tls: &mut ClientConnection, tcp: &mut Tcp) -> io::Result<()> {
    while tls.wants_write() {
        tls.write_tls(tcp)?;
    }
    Ok(())
}

/// Messages no larger than over TCP, and no more waiting to go out than a
/// connection may be behind by (with room for the messages' headers).
fn config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE))
        .max_write_buffer_size(MAX_BACKLOG + MAX_MESSAGE)
}

/// Connect to the server at `url`: ws://host:port/path, or wss://host/path
/// for a secure connection (as to a server online). This goes on in the
/// background, so the game can go on meanwhile: the connection, or why
/// there isn't one, comes through the channel within `timeout` (and however
/// long finding the host by name takes).
pub fn dial(url: &str, timeout: Duration) -> Receiver<Result<Connection, String>> {
    let roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    dial_trusting(url, timeout, roots)
}

/// `dial`, trusting only certificates from `roots` (a test's own, say).
pub(crate) fn dial_trusting(
    url: &str,
    timeout: Duration,
    roots: RootCertStore,
) -> Receiver<Result<Connection, String>> {
    let (tx, rx) = mpsc::channel();
    let url = url.to_string();
    std::thread::spawn(move || {
        // Whoever dialed may have stopped waiting.
        let _ = tx.send(connect(&url, Instant::now() + timeout, roots));
    });
    rx
}

fn connect(url: &str, deadline: Instant, roots: RootCertStore) -> Result<Connection, String> {
    let bad = || format!("not a server address: {url}");
    // Schemes are any case, but tungstenite only takes them in lowercase.
    let lowercase = match url.split_once("://") {
        Some((scheme, rest)) => format!("{}://{rest}", scheme.to_ascii_lowercase()),
        None => url.to_string(),
    };
    let uri: Uri = lowercase.parse().map_err(|_| bad())?;
    let secure = match uri.scheme_str() {
        Some("ws") => false,
        Some("wss") => true,
        _ => return Err(bad()),
    };
    let authority = uri.authority().ok_or_else(bad)?;
    let default_port = if secure { 443 } else { 80 };
    let port = match authority.port_u16() {
        Some(port) => port,
        // No port at all (rather than one that isn't one).
        None if authority.as_str().ends_with(authority.host()) => default_port,
        None => return Err(bad()),
    };
    // IPv6 addresses come in brackets in URLs.
    let host = authority.host().trim_matches(['[', ']']).to_string();
    let addresses = (host.as_str(), port).to_socket_addrs();
    let addresses: Vec<_> = addresses.map_err(|_| not_found(&host))?.collect();
    // The handshake has the time that's left.
    let tcp = Tcp {
        stream: reach(&host, &addresses, deadline)?,
        deadline: Some(deadline),
    };
    let stream = if secure {
        let name = ServerName::try_from(host.clone()).map_err(|_| bad())?;
        let tls = ClientConnection::new(tls_config(roots)?, name).map_err(|e| e.to_string())?;
        Stream::Tls(Box::new(tls), tcp)
    } else {
        Stream::Plain(tcp)
    };
    let (socket, _) = tungstenite::client::client_with_config(uri, stream, Some(config()))
        .map_err(|e| match e {
            // A read timed out (on Unix).
            HandshakeError::Interrupted(_) => too_long(&host),
            HandshakeError::Failure(e) => failed(&host, e),
        })?;
    Connection::ws(socket).map_err(|e| e.to_string())
}

/// How to make secure connections, trusting certificates from `roots`.
fn tls_config(roots: RootCertStore) -> Result<Arc<ClientConfig>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

/// A TCP connection to `host`, trying each of its `addresses` in turn.
/// Each has its share of the time, so one that leads nowhere (as an IPv6
/// address can, on a network without IPv6) leaves the rest time to answer.
pub(crate) fn reach(
    host: &str,
    addresses: &[SocketAddr],
    deadline: Instant,
) -> Result<TcpStream, String> {
    let mut why = not_found(host);
    for (i, address) in addresses.iter().enumerate() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(too_long(host));
        }
        let share = left / (addresses.len() - i) as u32;
        match TcpStream::connect_timeout(address, share) {
            Ok(stream) => return Ok(stream),
            Err(e) if e.kind() == ErrorKind::TimedOut => why = too_long(host),
            Err(e) => why = format!("couldn't connect to {host}: {e}"),
        }
    }
    Err(why)
}

/// Give up reading or writing `stream` after `after`.
fn set_timeouts(stream: &TcpStream, after: Duration) -> Result<(), String> {
    let set = stream
        .set_read_timeout(Some(after))
        .and_then(|()| stream.set_write_timeout(Some(after)));
    set.map_err(|e| e.to_string())
}

fn not_found(host: &str) -> String {
    format!("couldn't find {host}")
}

fn too_long(host: &str) -> String {
    format!("{host} took too long to answer")
}

/// What went wrong connecting to `host`, in words.
fn failed(host: &str, e: tungstenite::Error) -> String {
    use ErrorKind::{TimedOut, WouldBlock};
    match e {
        tungstenite::Error::Io(e) if matches!(e.kind(), WouldBlock | TimedOut) => too_long(host),
        tungstenite::Error::Http(response) => {
            format!("{host} isn't a game server ({})", response.status())
        }
        // Without tungstenite's "IO error" (for a bad certificate, say).
        tungstenite::Error::Io(e) => format!("couldn't connect to {host}: {e}"),
        e => format!("couldn't connect to {host}: {e}"),
    }
}

/// Take a connection to a server (`stream`, just accepted): read the web
/// request it begins with, and take up the WebSocket if that's what it asks
/// for, at one of the `paths` the server takes them at (anywhere else isn't
/// found). This waits for the request (a few seconds at most), so servers
/// take each connection on a thread of its own. Servers never do TLS:
/// online, whatever they run behind does it for them.
pub fn accept(stream: TcpStream, paths: &[&str]) -> Result<Request, String> {
    accept_within(stream, paths, REQUEST_TIMEOUT)
}

/// `accept`, giving the request `timeout` to arrive (a test's own, say).
pub(crate) fn accept_within(
    stream: TcpStream,
    paths: &[&str],
    timeout: Duration,
) -> Result<Request, String> {
    use ErrorKind::{Interrupted, TimedOut, WouldBlock};
    // What a listener that never waits takes may not either (on Windows).
    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
    let mut tcp = Tcp {
        stream,
        deadline: Some(Instant::now() + timeout),
    };
    let mut request = Vec::new();
    let size = loop {
        let mut buf = [0; 4096];
        match tcp.read(&mut buf) {
            Ok(0) => return Err("connection closed".into()),
            Ok(n) => request.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == Interrupted => continue,
            Err(e) if matches!(e.kind(), WouldBlock | TimedOut) => {
                return Err("never said what it wants".into())
            }
            Err(e) => return Err(e.to_string()),
        }
        let mut headers = [httparse::EMPTY_HEADER; 64];
        match httparse::Request::new(&mut headers).parse(&request) {
            Ok(httparse::Status::Complete(size)) => break size,
            Ok(httparse::Status::Partial) if request.len() < MAX_REQUEST => {}
            _ => return Err("not a web request".into()),
        }
    };
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut parsed = httparse::Request::new(&mut headers);
    parsed.parse(&request).map_err(|e| e.to_string())?;
    let target = parsed.path.unwrap_or("/");
    // Just the path: not the query, nor the scheme and host a proxy may
    // put first.
    let path = match target.parse::<Uri>() {
        Ok(uri) => uri.path().to_string(),
        Err(e) => return refuse(tcp.stream, "400 Bad Request", e.to_string()),
    };
    let upgrade = parsed.headers.iter().any(|h| {
        h.name.eq_ignore_ascii_case("upgrade") && h.value.eq_ignore_ascii_case(b"websocket")
    });
    if !upgrade {
        // The caller answers it, and has as long again to.
        set_timeouts(&tcp.stream, timeout)?;
        return Ok(Request::Http(tcp.stream, path));
    }
    if !paths.contains(&path.as_str()) {
        return refuse(tcp.stream, "404 Not Found", format!("nothing at {path}"));
    }
    // Answer it as tungstenite's own `accept` would, the request read.
    let mut answer = tungstenite::http::Request::builder()
        .method(parsed.method.unwrap_or("GET"))
        .uri(target);
    for h in parsed.headers.iter() {
        answer = answer.header(h.name, h.value);
    }
    let response = answer
        .body(())
        .map_err(|e| e.to_string())
        .and_then(|r| create_response(&r).map_err(|e| e.to_string()));
    let response = match response {
        Ok(response) => response,
        Err(why) => return refuse(tcp.stream, "400 Bad Request", why),
    };
    let mut head = Vec::new();
    write_response(&mut head, &response).map_err(|e| e.to_string())?;
    tcp.write_all(&head).map_err(|e| e.to_string())?;
    // Anything after the request is the WebSocket's.
    let rest = request[size..].to_vec();
    let stream = Stream::Plain(tcp);
    let socket = WebSocket::from_partially_read(stream, rest, Role::Server, Some(config()));
    let conn = Connection::ws(socket).map_err(|e| e.to_string())?;
    // Where a proxy says it's from: Cloudflare's own header (hosts behind
    // Cloudflare, as Render is, pass it on), or else the first address in
    // X-Forwarded-For.
    let header = |name: &str| {
        let mut headers = parsed.headers.iter();
        let value = headers.find(|h| h.name.eq_ignore_ascii_case(name))?.value;
        std::str::from_utf8(value).ok()
    };
    let forwarded = header("cf-connecting-ip")
        .or_else(|| header("x-forwarded-for")?.split(',').next())
        .and_then(|ip| ip.trim().parse().ok());
    Ok(Request::WebSocket(conn, path, forwarded))
}

/// Answer a web request that can't be taken with `status`, and say why it
/// wasn't.
fn refuse(stream: TcpStream, status: &str, why: String) -> Result<Request, String> {
    let _ = reply(stream, status, &why);
    Err(why)
}

/// Answer a web request (`Request::Http`) with `status` ("200 OK", say)
/// and `text`, and hang up.
pub fn reply(mut stream: TcpStream, status: &str, text: &str) -> io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        text.len()
    );
    stream.write_all(&[head.as_bytes(), text.as_bytes()].concat())
}
