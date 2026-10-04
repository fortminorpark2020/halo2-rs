//! WebSockets, as PCs reach the online service: a game dials the server on
//! a thread of its own (so the game goes on meanwhile), and the server
//! takes WebSockets and plain web requests (a browser's, a health check's)
//! on the same port.

use crate::conn::{Connection, MAX_MESSAGE};
use std::io::{self, ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use tungstenite::handshake::server::{create_response, write_response};
use tungstenite::handshake::HandshakeError;
use tungstenite::http::Uri;
use tungstenite::protocol::{Role, WebSocket, WebSocketConfig};

/// A web request must arrive this soon.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Longer requests (than any game's or browser's) are hostile.
const MAX_REQUEST: usize = 16 << 10;

/// What a connection to a server asked for.
pub enum Request {
    /// A WebSocket, at this path (`/live`, say).
    WebSocket(Connection, String),
    /// A page at this path (for a browser, or a health check): answer it
    /// with `reply`.
    Http(TcpStream, String),
}

/// Messages no larger than over TCP.
fn config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE))
}

/// Connect to the server at `url`: ws://host:port/path. This goes on in
/// the background, so the game can go on meanwhile: the connection, or why
/// there isn't one, comes through the channel within `timeout` (and however
/// long finding the host by name takes).
pub fn dial(url: &str, timeout: Duration) -> Receiver<Result<Connection, String>> {
    let (tx, rx) = mpsc::channel();
    let url = url.to_string();
    std::thread::spawn(move || {
        // Whoever dialed may have stopped waiting.
        let _ = tx.send(connect(&url, Instant::now() + timeout));
    });
    rx
}

fn connect(url: &str, deadline: Instant) -> Result<Connection, String> {
    let bad = || format!("not a server address: {url}");
    let uri: Uri = url.parse().map_err(|_| bad())?;
    if !uri.scheme_str().unwrap_or("").eq_ignore_ascii_case("ws") {
        return Err(bad());
    }
    let host = uri.host().ok_or_else(bad)?.to_string();
    let stream = reach(&host, uri.port_u16().unwrap_or(80), deadline)?;
    // The handshake has the time that's left.
    let left = deadline.saturating_duration_since(Instant::now());
    set_timeouts(&stream, left.max(Duration::from_millis(1)))?;
    let (socket, _) = tungstenite::client::client_with_config(uri, stream, Some(config()))
        .map_err(|e| match e {
            // A read timed out (on Unix).
            HandshakeError::Interrupted(_) => too_long(&host),
            HandshakeError::Failure(e) => failed(&host, e),
        })?;
    Connection::ws(socket).map_err(|e| e.to_string())
}

/// A TCP connection to `host`, trying each of its addresses in turn.
fn reach(host: &str, port: u16, deadline: Instant) -> Result<TcpStream, String> {
    // IPv6 addresses come in brackets in URLs.
    let name = host.trim_start_matches('[').trim_end_matches(']');
    let not_found = || format!("couldn't find {host}");
    let addresses = (name, port).to_socket_addrs().map_err(|_| not_found())?;
    let mut why = not_found();
    for address in addresses {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(too_long(host));
        }
        match TcpStream::connect_timeout(&address, left) {
            Ok(stream) => return Ok(stream),
            Err(e) if e.kind() == ErrorKind::TimedOut => why = too_long(host),
            Err(e) => why = format!("couldn't connect to {host}: {e}"),
        }
    }
    Err(why)
}

/// Give up reading or writing `stream` after `after` (during a handshake).
fn set_timeouts(stream: &TcpStream, after: Duration) -> Result<(), String> {
    let set = stream
        .set_read_timeout(Some(after))
        .and_then(|()| stream.set_write_timeout(Some(after)));
    set.map_err(|e| e.to_string())
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
        e => format!("couldn't connect to {host}: {e}"),
    }
}

/// Take a connection to a server (`stream`, just accepted): read the web
/// request it begins with, and take up the WebSocket if that's what it asks
/// for. This waits for the request (a few seconds at most), so servers
/// take each connection on a thread of its own. Servers never do TLS:
/// online, whatever they run behind does it for them.
pub fn accept(mut stream: TcpStream) -> Result<Request, String> {
    use ErrorKind::{Interrupted, TimedOut, WouldBlock};
    set_timeouts(&stream, REQUEST_TIMEOUT)?;
    let mut request = Vec::new();
    let size = loop {
        let mut buf = [0; 4096];
        match stream.read(&mut buf) {
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
    let path = parsed.path.unwrap_or("/").to_string();
    let upgrade = parsed.headers.iter().any(|h| {
        h.name.eq_ignore_ascii_case("upgrade") && h.value.eq_ignore_ascii_case(b"websocket")
    });
    if !upgrade {
        return Ok(Request::Http(stream, path));
    }
    // Answer it as tungstenite's own `accept` would, the request read.
    let mut answer = tungstenite::http::Request::builder()
        .method(parsed.method.unwrap_or("GET"))
        .uri(&path);
    for h in parsed.headers.iter() {
        answer = answer.header(h.name, h.value);
    }
    let response = answer
        .body(())
        .map_err(|e| e.to_string())
        .and_then(|r| create_response(&r).map_err(|e| e.to_string()))?;
    let mut head = Vec::new();
    write_response(&mut head, &response).map_err(|e| e.to_string())?;
    stream.write_all(&head).map_err(|e| e.to_string())?;
    // Anything after the request is the WebSocket's.
    let rest = request[size..].to_vec();
    let socket = WebSocket::from_partially_read(stream, rest, Role::Server, Some(config()));
    let conn = Connection::ws(socket).map_err(|e| e.to_string())?;
    Ok(Request::WebSocket(conn, path))
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
