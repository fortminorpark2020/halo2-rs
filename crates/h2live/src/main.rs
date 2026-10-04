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
//! shares one address between homes, so only a host online will do. A host
//! online sets PORT, and has its own way in.

use h2live::server::{log, Route, Server};
use h2net::Request;
use igd_next::{Gateway, PortMappingProtocol, SearchOptions};
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// The port and the data folder unless told otherwise.
const PORT: u16 = 47050;
const DATA: &str = "h2live-data";
/// Naps between polls: while things come in, and once nothing has for a
/// while (seconds).
const BUSY_NAP: Duration = Duration::from_millis(2);
const IDLE_NAP: Duration = Duration::from_millis(50);
const QUIET: f64 = 1.0;
/// New connections still saying what they want, at most; more are turned
/// away.
const ASKING: usize = 64;
/// How long to look for the router, to ask it for the port.
const UPNP_TIMEOUT: Duration = Duration::from_secs(3);

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
    /// Take on the `port=` and `data=` lines of h2live.txt's `text`. A data
    /// folder that isn't a whole path is in `folder`, the program's.
    fn read(&mut self, text: &str, folder: &Path) -> Result<(), String> {
        // Notepad may start the file with a byte order mark.
        for line in text.trim_start_matches('\u{feff}').lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "port" => self.port = port(value)?,
                "data" => self.data = folder.join(value),
                _ => {}
            }
        }
        Ok(())
    }
}

fn port(text: &str) -> Result<u16, String> {
    text.trim()
        .parse()
        .map_err(|_| format!("{text} isn't a port number"))
}

fn main() {
    match std::panic::catch_unwind(run) {
        Ok(Ok(())) => return,
        Ok(Err(why)) => log(format_args!("FAILED: {why}")),
        // The panic said why.
        Err(_) => {}
    }
    // Keep the window open when started with a double-click.
    if cfg!(windows) {
        println!("Press Enter to close.");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
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
    let router = if settings.upnp { open_port(port) } else { None };

    serve(&listener, &mut server, &stop, settings.hosted);

    // Accounts are saved as they change, so there's nothing left to save.
    let players = server.players_online();
    drop(server);
    if let Some(router) = router {
        if let Err(e) = router.remove_port(PortMappingProtocol::TCP, port) {
            log(format_args!("the router kept port {port} open: {e}"));
        }
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
            read.map_err(|why| format!("{}: {why}", file.display()))?;
            log(format_args!("settings from {}", file.display()));
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
    let mut asking = 0;
    // When something last came in.
    let mut heard = f64::NEG_INFINITY;
    while !stop.load(Ordering::SeqCst) {
        let now = start.elapsed().as_secs_f64();
        let mut busy = false;
        for _ in 0..ASKING {
            let Ok((stream, from)) = listener.accept() else {
                break;
            };
            busy = true;
            if asking == ASKING {
                continue;
            }
            asking += 1;
            let asked = asked.clone();
            std::thread::spawn(move || {
                let _ = asked.send((h2net::accept(stream), from.ip()));
            });
        }
        for (request, from) in asks.try_iter() {
            asking -= 1;
            busy = true;
            match request {
                Ok(Request::WebSocket(conn, path, forwarded)) => {
                    let ip = forwarded.filter(|_| hosted).unwrap_or(from);
                    match path.as_str() {
                        "/live" => server.accept(conn, Route::Live, ip, now),
                        "/link" => server.accept(conn, Route::Link, ip, now),
                        _ => log(format_args!("{ip}: nothing at {path}")),
                    }
                }
                Ok(Request::Http(stream, path)) => answer(stream, &path, server),
                Err(why) => log(format_args!("{from}: {why}")),
            }
        }
        busy |= server.poll(now);
        if busy {
            heard = now;
        }
        let quiet = now - heard >= QUIET;
        std::thread::sleep(if quiet { IDLE_NAP } else { BUSY_NAP });
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
/// reach the server. The router, if it did (to close the port again).
fn open_port(port: u16) -> Option<Gateway> {
    let Some(IpAddr::V4(local)) = h2net::local_ip() else {
        log("this PC isn't on a network");
        return None;
    };
    let forward = || {
        println!("FORWARD TCP {port} TO {local}");
        println!("(the router didn't open the port itself: forward it in the router's settings)");
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
            println!("CGNAT: USE THE HOSTED OPTION");
            println!("(the internet provider shares one address between homes, so players can't reach this PC)");
            return None;
        }
        Err(e) => {
            log(format_args!("the router didn't say its address: {e}"));
            forward();
            return None;
        }
    };
    let to = SocketAddr::new(local.into(), port);
    // Open until h2live stops, and closes it.
    match router.add_port(PortMappingProtocol::TCP, port, to, 0, "h2live") {
        Ok(()) => {
            println!("READY: ws://{external}:{port}");
            println!("(players sign in at this address)");
            Some(router)
        }
        Err(e) => {
            log(format_args!("the router didn't open port {port}: {e}"));
            forward();
            None
        }
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

    #[test]
    fn h2live_txt_sets_the_port_and_data_folder() {
        let folder = Path::new("/games/h2live");
        let mut s = Settings::default();
        let text = "\u{feff}# John's server\nport = 47060\ndata=saves\nnonsense\nsecret=no\n";
        s.read(text, folder).unwrap();
        assert_eq!((s.port, s.data), (47060, folder.join("saves")));
        assert_eq!(s.secret, None);
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
}
