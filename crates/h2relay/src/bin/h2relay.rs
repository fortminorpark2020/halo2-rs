//! A relay server on its own, for tests on one PC (two launchers and the
//! relay all on 127.0.0.1, say). h2live runs the same relay beside its
//! sign-in port, so this isn't needed for real play.
//!
//! ```text
//! h2relay                  # 0.0.0.0:47050
//! h2relay 47051            # 0.0.0.0:47051
//! h2relay 127.0.0.1:47051  # this PC only
//! ```
//!
//! Rooms are open: any room token a launcher names is made, and anyone who
//! knows a room's token can join it. So run it on one PC or a home network
//! only, and don't forward its port: h2live's own relay admits only the
//! rooms it issues for matches. Ctrl+C stops it.

use h2relay::{stdout_logger, RelayServer, ServerConfig, DEFAULT_PORT, RELAY_PROTOCOL};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let arg = std::env::args().nth(1);
    let addr = match arg.as_deref() {
        None => SocketAddr::from((Ipv4Addr::UNSPECIFIED, DEFAULT_PORT)),
        Some("-h" | "--help") => {
            println!(
                "usage: h2relay [port | address:port]   (0.0.0.0:{DEFAULT_PORT} unless given)"
            );
            return;
        }
        Some(arg) => match (arg.parse::<u16>(), arg.parse::<SocketAddr>()) {
            (Ok(port), _) => SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)),
            (_, Ok(addr)) => addr,
            _ => {
                eprintln!("h2relay: {arg:?} isn't a port or an address:port");
                std::process::exit(2);
            }
        },
    };
    let server = match RelayServer::bind(addr, ServerConfig::default()) {
        Ok(server) => server,
        Err(e) => {
            eprintln!("h2relay: can't listen on UDP {addr}: {e}");
            std::process::exit(1);
        }
    };
    let start = Instant::now();
    // Written by a thread of its own: a console with text selected in it
    // (on Windows) holds up its output, and must not hold up the relay.
    let out = stdout_logger();
    let log = Arc::new(move |line: &str| {
        let t = start.elapsed().as_secs_f64();
        out(&format!("[{t:9.3}] {line}"));
    });
    log(&format!(
        "h2relay, relay protocol {RELAY_PROTOCOL}, on UDP {} with open rooms \
         (for tests: don't forward this port; Ctrl+C stops)",
        server.local_addr()
    ));
    server.with_log(log).run();
}
