//! Finding games: hosts broadcast a small announcement every second, and
//! every running game listens for them.

use crate::{BEACON_PORT, MAGIC, PROTOCOL};
use h2sim::game::{Reader, Writer};
use socket2::{Domain, Protocol, Socket, Type};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

const BEACON_EVERY: Duration = Duration::from_secs(1);
/// Games not heard from for this long are gone.
const FORGET_AFTER: Duration = Duration::from_secs(4);

/// A game someone is hosting on the network.
#[derive(Debug, Clone, PartialEq)]
pub struct LanGame {
    /// Where to join it.
    pub address: SocketAddr,
    pub map: String,
    /// The host PC's name.
    pub computer: String,
    pub players: usize,
    session: u64,
    seen: Instant,
}

impl LanGame {
    /// A game at a known address, without having heard it announced.
    pub fn at(address: SocketAddr, map: &str) -> LanGame {
        LanGame {
            address,
            map: map.to_string(),
            computer: address.ip().to_string(),
            players: 0,
            session: 0,
            seen: Instant::now(),
        }
    }
}

pub(crate) struct Beacon {
    socket: Option<UdpSocket>,
    session: u64,
    port: u16,
    next: Instant,
}

impl Beacon {
    pub fn new(session: u64, port: u16) -> Beacon {
        let socket = UdpSocket::bind(("0.0.0.0", 0)).and_then(|s| {
            s.set_broadcast(true)?;
            s.set_nonblocking(true)?;
            Ok(s)
        });
        if let Err(e) = &socket {
            println!("lan: can't announce the game: {e}");
        }
        Beacon {
            socket: socket.ok(),
            session,
            port,
            next: Instant::now(),
        }
    }

    pub fn announce(&mut self, map: &str, computer: &str, players: u8) {
        let Some(s) = &self.socket else { return };
        if Instant::now() < self.next {
            return;
        }
        self.next = Instant::now() + BEACON_EVERY;
        let mut w = Writer::default();
        w.u32(MAGIC);
        w.u32(PROTOCOL);
        w.u64(self.session);
        w.u16(self.port);
        w.u8(players);
        w.str(map);
        w.str(computer);
        // The whole network, and this PC (for games running side by side).
        for ip in [Ipv4Addr::BROADCAST, Ipv4Addr::LOCALHOST] {
            let _ = s.send_to(&w.0, (ip, BEACON_PORT));
        }
    }
}

fn read_beacon(data: &[u8], from: IpAddr) -> Option<LanGame> {
    let mut r = Reader::new(data);
    if r.u32().ok()? != MAGIC || r.u32().ok()? != PROTOCOL {
        return None;
    }
    let session = r.u64().ok()?;
    let port = r.u16().ok()?;
    let players = r.u8().ok()? as usize;
    let map = r.str().ok()?;
    let computer = r.str().ok()?;
    Some(LanGame {
        address: SocketAddr::new(from, port),
        map,
        computer,
        players,
        session,
        seen: Instant::now(),
    })
}

/// Listens for games on the network.
pub struct Browser {
    socket: Option<UdpSocket>,
    own: u64,
    games: Vec<LanGame>,
}

fn listen() -> std::io::Result<UdpSocket> {
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    // Several games on one PC can all listen.
    s.set_reuse_address(true)?;
    s.set_broadcast(true)?;
    s.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, BEACON_PORT)).into())?;
    s.set_nonblocking(true)?;
    Ok(s.into())
}

impl Browser {
    /// `own` is this game's session, so it doesn't list itself.
    pub fn new(own: u64) -> Browser {
        let socket = listen();
        if let Err(e) = &socket {
            println!("lan: can't look for games: {e}");
        }
        Browser {
            socket: socket.ok(),
            own,
            games: Vec::new(),
        }
    }

    /// Games heard from recently, oldest find first.
    pub fn poll(&mut self) -> &[LanGame] {
        let mut buf = [0u8; 1024];
        if let Some(s) = &self.socket {
            while let Ok((n, from)) = s.recv_from(&mut buf) {
                let Some(game) = read_beacon(&buf[..n], from.ip()) else {
                    continue;
                };
                if game.session == self.own {
                    continue;
                }
                match self.games.iter_mut().find(|g| g.session == game.session) {
                    // Prefer the network address over this PC's loopback.
                    Some(g) if game.address.ip().is_loopback() && !g.address.ip().is_loopback() => {
                        g.seen = game.seen;
                    }
                    Some(g) => *g = game,
                    None => self.games.push(game),
                }
            }
        }
        self.games.retain(|g| g.seen.elapsed() < FORGET_AFTER);
        &self.games
    }
}

/// This PC's address on the local network, if it has one.
pub fn local_ip() -> Option<IpAddr> {
    // Connecting a UDP socket sends nothing; it only picks the route.
    let s = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    s.connect(("192.168.0.1", 9)).ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_unspecified()).then_some(ip)
}
