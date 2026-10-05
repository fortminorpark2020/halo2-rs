//! h2live, the online service's program, for a host online (Render, say) or
//! a PC at home. It listens on one port for games, which come as WebSockets
//! (`/live` for a PC's control link, `/link` for a relayed game), and for
//! web requests: `/health` for a host's health check, and `/` for a page
//! saying how many are on.
//!
//! Its settings come from the environment: PORT (47050 unless set),
//! H2LIVE_DATA (the folder accounts are kept in, `h2live-data` unless set),
//! H2LIVE_SECRET (what stat cards are signed with; without it, a key kept in
//! `secret.txt` there), and H2LIVE_UPNP=0 to leave the router alone. So a
//! double-click is enough at home, an `h2live.txt` next to the program can
//! hold `port=` and `data=` lines too.
//!
//! At home it asks the router to pass the port on to this PC (UPnP), and
//! says how players reach it: READY with the address, FORWARD when the
//! router has to be told by hand, or CGNAT when the internet provider
//! shares one address between homes, so only a host online will do. The
//! router opens it for an hour at a time, asked again while h2live runs,
//! so it closes by itself however h2live stops. A host online sets PORT,
//! and has its own way in.

use h2live::server::{flush_log, log, log_in_background, say, Route, Server};
use h2net::Request;
use igd_next::{AddPortError, Gateway, PortMappingProtocol, SearchOptions};
use rustix::event::{PollFd, PollFlags, Timespec};
use rustix::io::Errno;
use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The port and the data folder unless told otherwise.
const PORT: u16 = 47050;
const DATA: &str = "h2live-data";
/// Polls are this far apart at least, so a busy server reads what came
/// meanwhile all at once; and at most this, with nothing coming in.
const NAP: Duration = Duration::from_millis(5);
const IDLE: Duration = Duration::from_millis(50);
/// Where games come in, as WebSockets: anywhere else isn't found.
const PATHS: &[&str] = &["/live", "/link"];
/// New connections still saying what they want, at most: more wait their
/// turn (the system holds them), so no one is turned away for want of room.
const ASKING: usize = 64;
/// At most this many of them from any one address, so one can't keep
/// everyone else waiting: more from it are turned away. (Not on a host
/// online, where they all come from its proxy, and it passes on only
/// whole requests.)
const ASKING_EACH: usize = 8;
/// How long to look for the router, to ask it for the port.
const UPNP_TIMEOUT: Duration = Duration::from_secs(3);
/// How long the router keeps the port open (seconds) unless asked again,
/// so it closes by itself if h2live stops without closing it (its window
/// closed, say); and how often it's asked again, well before then.
const LEASE: u32 = 3600;
const RENEW: Duration = Duration::from_secs(20 * 60);

/// How the program was told to run.
struct Settings {
    port: u16,
    data: PathBuf,
    secret: Option<String>,
    /// On a host online (PORT was set): connections come through its proxy,
    /// so they're from where the proxy says, and there's no router to ask.
    hosted: bool,
    upnp: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            port: PORT,
            data: PathBuf::from(DATA),
            secret: None,
            hosted: false,
            upnp: true,
        }
    }
}

impl Settings {
    /// Take on the `port=` and `data=` lines of h2live.txt's `text`, in any
    /// case. A data folder that isn't a whole path is in `folder`, the
    /// program's. The lines it didn't take (but for blank ones and #
    /// comments), to say so.
    fn read<'a>(&mut self, text: &'a str, folder: &Path) -> Result<Vec<&'a str>, String> {
        let mut ignored = Vec::new();
        // Notepad may start the file with a byte order mark.
        for line in text.trim_start_matches('\u{feff}').lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                ignored.push(line);
                continue;
            };
            let value = value.trim();
            match key.trim().to_ascii_lowercase().as_str() {
                "port" => self.port = port(value)?,
                "data" => self.data = folder.join(value),
                _ => ignored.push(line),
            }
        }
        Ok(ignored)
    }
}

fn port(text: &str) -> Result<u16, String> {
    text.trim()
        .parse()
        .map_err(|_| format!("{text} isn't a port number"))
}

fn main() {
    // So a console that stops taking what's said (one with text selected,
    // on Windows) holds no game up.
    log_in_background();
    match std::panic::catch_unwind(run) {
        Ok(Ok(())) => {
            flush_log();
            return;
        }
        Ok(Err(why)) => log(format_args!("FAILED: {why}")),
        // The panic said why.
        Err(_) => {}
    }
    // Keep the window open when started with a double-click.
    if cfg!(windows) {
        say("Press Enter to close.");
        flush_log();
        let _ = std::io::stdin().read_line(&mut String::new());
    }
    flush_log();
    std::process::exit(1);
}

fn run() -> Result<(), String> {
    log(format_args!("h2live, protocol {}", h2net::PROTOCOL));
    let settings = settings()?;
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, settings.port)).map_err(|e| {
        let port = settings.port;
        match e.kind() {
            ErrorKind::AddrInUse => format!("port {port} is in use (is h2live running already?)"),
            _ => format!("can't listen on port {port}: {e}"),
        }
    })?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    // As it is: port 0 is any that's free.
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let mut server = Server::open(&settings.data, settings.secret.as_deref())?;
    let data = std::path::absolute(&settings.data).unwrap_or(settings.data);
    log(format_args!("accounts are kept in {}", data.display()));
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    ctrlc::set_handler(move || stopping.store(true, Ordering::SeqCst))
        .map_err(|e| e.to_string())?;
    log(format_args!("listening on port {port} (Ctrl+C stops)"));
    let open = if settings.upnp { open_port(port) } else { None };

    serve(&listener, &mut server, &stop, settings.hosted);

    // Accounts are saved as they change, so there's nothing left to save.
    let players = server.players_online();
    drop(server);
    if let Some(open) = open {
        open.close();
    }
    log(format_args!("stopped, with {players} players online"));
    Ok(())
}

/// The settings: from the environment, or else h2live.txt next to the
/// program, or else the defaults.
fn settings() -> Result<Settings, String> {
    let mut settings = Settings::default();
    let exe = std::env::current_exe().ok();
    if let Some(folder) = exe.as_deref().and_then(Path::parent) {
        let file = folder.join("h2live.txt");
        if let Ok(text) = std::fs::read_to_string(&file) {
            let read = settings.read(&text, folder);
            let ignored = read.map_err(|why| format!("{}: {why}", file.display()))?;
            log(format_args!("settings from {}", file.display()));
            for line in ignored {
                log(format_args!(
                    "ignored {line:?} (only port= and data= lines count)"
                ));
            }
        }
    }
    let env = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
    if let Some(p) = env("PORT") {
        settings.port = port(&p).map_err(|why| format!("PORT: {why}"))?;
        settings.hosted = true;
    }
    if let Some(data) = env("H2LIVE_DATA") {
        settings.data = data.into();
    }
    settings.secret = env("H2LIVE_SECRET");
    settings.upnp = !settings.hosted && env("H2LIVE_UPNP").as_deref() != Some("0");
    Ok(settings)
}

/// Serve until `stop`: take connections, each saying what it wants on a
/// thread of its own (so a slow one holds no one up), answer web requests,
/// and hand games to `server`. A host's proxy says where each is from.
fn serve(listener: &TcpListener, server: &mut Server, stop: &AtomicBool, hosted: bool) {
    let start = Instant::now();
    let (asked, asks) = mpsc::channel();
    // Connections still saying what they want: how many, how many from
    // each address, and the addresses with more turned away (said once).
    let mut asking = 0;
    let mut from_each = HashMap::<IpAddr, usize>::new();
    let mut turned_away = HashSet::new();
    while !stop.load(Ordering::SeqCst) {
        let now = start.elapsed().as_secs_f64();
        while asking < ASKING {
            let Ok((stream, from)) = listener.accept() else {
                break;
            };
            let ip = from.ip();
            let each = from_each.entry(ip).or_default();
            if *each == ASKING_EACH && !hosted {
                if turned_away.insert(ip) {
                    log(format_args!(
                        "{ip}: too many connections at once, more turned away"
                    ));
                }
                continue;
            }
            *each += 1;
            asking += 1;
            let asked = asked.clone();
            std::thread::spawn(move || {
                let _ = asked.send((h2net::accept(stream, PATHS), ip));
            });
        }
        for (request, from) in asks.try_iter() {
            asking -= 1;
            if let Some(each) = from_each.get_mut(&from) {
                *each -= 1;
                if *each == 0 {
                    from_each.remove(&from);
                    turned_away.remove(&from);
                }
            }
            match request {
                Ok(Request::WebSocket(conn, path, forwarded)) => {
                    let ip = forwarded.filter(|_| hosted).unwrap_or(from);
                    let route = match path.as_str() {
                        "/live" => Route::Live,
                        // The only other path in PATHS.
                        _ => Route::Link,
                    };
                    server.accept(conn, route, ip, now);
                }
                Ok(Request::Http(stream, path)) => answer(stream, &path, server),
                Err(why) => log(format_args!("{from}: {why}")),
            }
        }
        server.poll(now);
        std::thread::sleep(NAP);
        // Connections still saying what they want come by channel, which
        // can't be waited on.
        let idle = if asking > 0 { Duration::ZERO } else { IDLE };
        wait(listener, server, idle);
    }
}

/// Wait for something to come in, or a connection, for `idle` at most.
fn wait(listener: &TcpListener, server: &Server, idle: Duration) {
    let ready = PollFlags::IN;
    let mut fds: Vec<PollFd> = server.streams().map(|s| PollFd::new(s, ready)).collect();
    fds.push(PollFd::new(listener, ready));
    let timeout = Timespec::try_from(idle).ok();
    match rustix::event::poll(&mut fds, timeout.as_ref()) {
        // Ctrl+C, say.
        Ok(_) | Err(Errno::INTR) => {}
        Err(e) => {
            // Not to spin, whatever it was.
            log(format_args!("waiting: {e}"));
            std::thread::sleep(idle);
        }
    }
}

/// Answer a web request: a host's health check, or someone looking at the
/// server's page.
fn answer(stream: TcpStream, path: &str, server: &Server) {
    let (status, text) = match path {
        "/health" => ("200 OK", "OK".to_string()),
        "/" => ("200 OK", page(server)),
        _ => ("404 Not Found", "NOT FOUND".to_string()),
    };
    // Someone who went away meanwhile is no matter.
    let _ = h2net::reply(stream, status, &text);
}

/// The server's page: how many are on, and playing.
fn page(server: &Server) -> String {
    let count =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    format!(
        "H2LIVE\n{} ONLINE\n{} IN PROGRESS\n{}\n",
        count(server.players_online(), "PLAYER", "PLAYERS"),
        count(server.matches_in_progress(), "MATCH", "MATCHES"),
        count(server.custom_games(), "CUSTOM GAME", "CUSTOM GAMES"),
    )
}

/// Ask the router to pass `port` on to this PC (UPnP), and say how players
/// reach the server. The port, if the router opened it (to close again).
fn open_port(port: u16) -> Option<OpenPort> {
    let Some(IpAddr::V4(local)) = h2net::local_ip() else {
        log("this PC isn't on a network");
        return None;
    };
    let forward = || {
        say(format_args!("FORWARD TCP {port} TO {local}"));
        say("(the router didn't open the port itself: forward it in the router's settings)");
    };
    let mut options = SearchOptions::default();
    options.timeout = Some(UPNP_TIMEOUT);
    options.single_search_timeout = Some(UPNP_TIMEOUT);
    let router = match igd_next::search_gateway(options) {
        Ok(router) => router,
        Err(e) => {
            log(format_args!("no router answered UPnP: {e}"));
            forward();
            return None;
        }
    };
    let external = match router.get_external_ip() {
        Ok(IpAddr::V4(ip)) if public(ip) => ip,
        Ok(ip) => {
            log(format_args!("the router's own address is {ip}"));
            say("CGNAT: USE THE HOSTED OPTION");
            say("(the internet provider shares one address between homes, so players can't reach this PC)");
            return None;
        }
        Err(e) => {
            log(format_args!("the router didn't say its address: {e}"));
            forward();
            return None;
        }
    };
    let to = SocketAddr::new(local.into(), port);
    match OpenPort::open(router, port, to, RENEW) {
        Ok(open) => {
            say(format_args!("READY: ws://{external}:{port}"));
            say("(players sign in at this address)");
            Some(open)
        }
        Err(e) => {
            log(format_args!("the router didn't open port {port}: {e}"));
            forward();
            None
        }
    }
}

/// A port the router passes on to this PC: kept open on a thread of its
/// own, which asks the router again every so often, until `close`.
struct OpenPort {
    stop: mpsc::Sender<()>,
    keeper: JoinHandle<()>,
}

impl OpenPort {
    /// Ask `router` to pass `port` on to `to` for a while (LEASE), and
    /// again every `renew`; or for good, if that's all the router does.
    fn open(
        router: Gateway,
        port: u16,
        to: SocketAddr,
        renew: Duration,
    ) -> Result<OpenPort, AddPortError> {
        let tcp = PortMappingProtocol::TCP;
        let lease = match router.add_port(tcp, port, to, LEASE, "h2live") {
            Ok(()) => LEASE,
            Err(AddPortError::OnlyPermanentLeasesSupported) => {
                router.add_port(tcp, port, to, 0, "h2live")?;
                0
            }
            Err(e) => return Err(e),
        };
        let (stop, stopped) = mpsc::channel();
        let keeper = std::thread::spawn(move || {
            while lease != 0 && stopped.recv_timeout(renew) == Err(RecvTimeoutError::Timeout) {
                if let Err(e) = router.add_port(tcp, port, to, lease, "h2live") {
                    log(format_args!("the router didn't keep port {port} open: {e}"));
                }
            }
            // Until told to stop.
            let _ = stopped.recv();
            if let Err(e) = router.remove_port(tcp, port) {
                log(format_args!("the router kept port {port} open: {e}"));
            }
        });
        Ok(OpenPort { stop, keeper })
    }

    /// Close the port again.
    fn close(self) {
        drop(self.stop);
        let _ = self.keeper.join();
    }
}

/// `ip` can be reached from anywhere: it isn't a private network's, nor
/// one an internet provider shares between homes (CGNAT's, in
/// 100.64.0.0/10).
fn public(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    let shared = a == 100 && b & 0xc0 == 64;
    let private = ip.is_private() || ip.is_loopback() || ip.is_link_local();
    !(private || shared || ip.is_unspecified())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// How long anything may take.
    const WAIT: Duration = Duration::from_secs(10);

    #[test]
    fn h2live_txt_sets_the_port_and_data_folder() {
        let folder = Path::new("/games/h2live");
        let mut s = Settings::default();
        // As Notepad saves it, in any case.
        let text = "\u{feff}# John's server\r\nPort = 47060\r\nDATA=saves\r\n\r\n\
                    nonsense\r\nsecret=no\r\nport: 47061\r\n";
        let ignored = s.read(text, folder).unwrap();
        assert_eq!((s.port, s.data), (47060, folder.join("saves")));
        assert_eq!(s.secret, None);
        // What it didn't take, to say so.
        assert_eq!(ignored, ["nonsense", "secret=no", "port: 47061"]);
        // A whole path is as it is.
        let mut s = Settings::default();
        s.read("data=/srv/h2live\n", folder).unwrap();
        assert_eq!(s.data, Path::new("/srv/h2live"));
        let why = Settings::default().read("port=forty\n", folder).err();
        assert_eq!(why.as_deref(), Some("forty isn't a port number"));
    }

    #[test]
    fn private_and_shared_addresses_arent_public() {
        for ip in [
            "192.168.1.20",
            "10.0.0.1",
            "172.16.5.4",
            "100.64.0.1",
            "100.127.255.254",
            "127.0.0.1",
            "169.254.1.1",
            "0.0.0.0",
        ] {
            assert!(!public(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["203.0.113.5", "100.128.0.1", "100.63.255.255", "8.8.8.8"] {
            assert!(public(ip.parse().unwrap()), "{ip}");
        }
    }

    /// `serve` on a thread of its own and a port of its own on this PC,
    /// behind a host's proxy (`hosted`) or not, until dropped.
    struct Serving {
        address: SocketAddr,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
        data: PathBuf,
    }

    impl Serving {
        fn start(name: &str, hosted: bool) -> Serving {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let id = std::process::id();
            let data = std::env::temp_dir().join(format!("h2live-serve-{name}-{id}"));
            let mut server = Server::open(&data, Some("test")).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let stopping = stop.clone();
            let thread =
                std::thread::spawn(move || serve(&listener, &mut server, &stopping, hosted));
            Serving {
                address,
                stop,
                thread: Some(thread),
                data,
            }
        }

        /// A connection to it from `from`, one of this PC's own addresses
        /// (any of 127.0.0.1 to 127.255.255.254).
        fn connect(&self, from: [u8; 4]) -> TcpStream {
            use socket2::{Domain, Socket, Type};
            let socket = Socket::new(Domain::IPV4, Type::STREAM, None).unwrap();
            socket.bind(&SocketAddr::from((from, 0)).into()).unwrap();
            socket.connect(&self.address.into()).unwrap();
            socket.into()
        }
    }

    impl Drop for Serving {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            let _ = self.thread.take().unwrap().join();
            let _ = std::fs::remove_dir_all(&self.data);
        }
    }

    /// The answer to a health check over `stream`, however long it takes
    /// (up to WAIT): nothing, if it was hung up on.
    fn health(mut stream: TcpStream) -> String {
        stream.set_read_timeout(Some(WAIT)).unwrap();
        write!(stream, "GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        let mut answer = String::new();
        let _ = stream.read_to_string(&mut answer);
        answer
    }

    #[test]
    fn connections_saying_nothing_turn_no_one_away() {
        // Behind a host's proxy, where they all come from one address.
        let serving = Serving::start("crowd", true);
        let mut crowd: Vec<_> = (0..ASKING)
            .map(|_| serving.connect([127, 0, 0, 1]))
            .collect();
        // A health check waits its turn...
        let check = serving.connect([127, 0, 0, 1]);
        let checking = std::thread::spawn(move || health(check));
        std::thread::sleep(Duration::from_millis(200));
        // ...until one of them goes.
        crowd.pop();
        let answer = checking.join().unwrap();
        assert!(answer.starts_with("HTTP/1.1 200 OK\r\n"), "{answer:?}");
    }

    #[test]
    fn one_address_cant_keep_everyone_else_waiting() {
        let serving = Serving::start("crowded", false);
        let crowd: Vec<_> = (0..ASKING)
            .map(|_| serving.connect([127, 0, 0, 2]))
            .collect();
        // All but the first few from there are turned away at once...
        let mut last = &crowd[ASKING - 1];
        last.set_read_timeout(Some(WAIT)).unwrap();
        let start = Instant::now();
        assert!(matches!(last.read(&mut [0]), Ok(0)));
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "{:?}",
            start.elapsed()
        );
        // ...so a health check from elsewhere is answered at once.
        let start = Instant::now();
        let answer = health(serving.connect([127, 0, 0, 1]));
        assert!(answer.starts_with("HTTP/1.1 200 OK\r\n"), "{answer:?}");
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "{:?}",
            start.elapsed()
        );
    }
    /// What a router is asked over UPnP: the action, and the lease asked
    /// for (seconds).
    type Asked = (String, Option<u32>);

    const SERVICE: &str = "urn:schemas-upnp-org:service:WANIPConnection:1";

    /// A router as UPnP sees it, that opens ports for a while at a time
    /// (`leases`), or else only for good. What it's asked comes through the
    /// channel.
    fn router(leases: bool) -> (Gateway, mpsc::Receiver<Asked>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, asked) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let mut request = String::new();
                while !request.ends_with("</s:Envelope>") {
                    let mut buf = [0; 4096];
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0, "{request}");
                    request.push_str(&String::from_utf8_lossy(&buf[..n]));
                }
                let action = request.split_once('#').unwrap().1;
                let action = action.split('"').next().unwrap().to_string();
                let lease = request.split_once("<NewLeaseDuration>");
                let lease = lease.and_then(|(_, rest)| rest.split('<').next()?.parse().ok());
                let refused = !leases && lease.is_some_and(|l| l != 0);
                let done = action == "DeletePortMapping";
                let body = if refused {
                    "<s:Fault><faultcode>s:Client</faultcode><faultstring>UPnPError</faultstring>\
                     <detail><UPnPError xmlns=\"urn:schemas-upnp-org:control-1-0\">\
                     <errorCode>725</errorCode>\
                     <errorDescription>OnlyPermanentLeasesSupported</errorDescription>\
                     </UPnPError></detail></s:Fault>"
                        .to_string()
                } else {
                    format!("<u:{action}Response xmlns:u=\"{SERVICE}\"/>")
                };
                let status = if refused {
                    "500 Internal Server Error"
                } else {
                    "200 OK"
                };
                let _ = tx.send((action, lease));
                let envelope = format!(
                    "<?xml version=\"1.0\"?><s:Envelope \
                     xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\">\
                     <s:Body>{body}</s:Body></s:Envelope>"
                );
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: text/xml\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n{envelope}",
                    envelope.len()
                );
                if done {
                    break;
                }
            }
        });
        let args = |names: &[&str]| names.iter().map(|n| n.to_string()).collect();
        let add = [
            "NewRemoteHost",
            "NewExternalPort",
            "NewProtocol",
            "NewInternalPort",
            "NewInternalClient",
            "NewEnabled",
            "NewPortMappingDescription",
            "NewLeaseDuration",
        ];
        let delete = ["NewRemoteHost", "NewExternalPort", "NewProtocol"];
        let gateway = Gateway {
            addr,
            root_url: "/rootDesc.xml".into(),
            control_url: "/ctl/IPConn".into(),
            control_schema_url: "/WANIPCn.xml".into(),
            control_schema: HashMap::from([
                ("AddPortMapping".into(), args(&add)),
                ("DeletePortMapping".into(), args(&delete)),
            ]),
            service_type: SERVICE.into(),
        };
        (gateway, asked)
    }

    #[test]
    fn the_router_opens_the_port_for_a_while_at_a_time() {
        let (router, asked) = router(true);
        let to = SocketAddr::from(([192, 168, 1, 20], PORT));
        let open = OpenPort::open(router, PORT, to, Duration::from_millis(20)).unwrap();
        // For a while, and asked again before then...
        let add: Asked = ("AddPortMapping".into(), Some(LEASE));
        for _ in 0..3 {
            assert_eq!(asked.recv_timeout(WAIT), Ok(add.clone()));
        }
        // ...until h2live stops, and closes it.
        open.close();
        let rest: Vec<_> = asked.try_iter().collect();
        let (last, renewed) = rest.split_last().unwrap();
        assert_eq!(*last, ("DeletePortMapping".into(), None));
        assert!(renewed.iter().all(|a| *a == add), "{rest:?}");
    }

    #[test]
    fn routers_that_only_open_ports_for_good_are_asked_for_that() {
        let (router, asked) = router(false);
        let to = SocketAddr::from(([192, 168, 1, 20], PORT));
        let open = OpenPort::open(router, PORT, to, Duration::from_millis(20)).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        open.close();
        let asked: Vec<_> = asked.try_iter().collect();
        let expected: [Asked; 3] = [
            ("AddPortMapping".into(), Some(LEASE)),
            ("AddPortMapping".into(), Some(0)),
            ("DeletePortMapping".into(), None),
        ];
        assert_eq!(asked, expected);
    }
}
