//! Hosting: accept PCs that join, give each of their players a Spartan, run
//! their controls through the game and send everyone the result. Between
//! games the host is in its lobby: joined PCs wait there, and say hello
//! again (with the map loaded) when a game starts.

use crate::conn::Connection;
use crate::discovery::Beacon;
use crate::{kind, Lobby, ANY_TEAM, MAGIC, PROTOCOL};
use h2sim::game::{guest_name, Event, Look, Reader, Writer};
use h2sim::{Command, Game};
use socket2::{Domain, Protocol, Socket, Type};
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::time::{Duration, Instant};

/// Joining PCs must say hello this soon.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// Things the host's game should show or act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    /// A PC came into the lobby.
    Arrived { computer: String },
    /// A PC joined the game with these players.
    Joined {
        computer: String,
        players: Vec<usize>,
    },
    /// Someone on a joined PC started playing in splitscreen.
    Added { player: usize },
    /// A joined PC's splitscreen player stopped playing; no one controls
    /// their Spartan now.
    Removed { player: usize },
    /// A PC left (or was dropped); no one controls these Spartans now.
    Left {
        computer: String,
        players: Vec<usize>,
        reason: String,
    },
}

/// Who a PC joining through `Host::add_connection` is, as the online
/// service checked it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub account: u64,
    pub gamertag: String,
    /// Their level (1-50) in what's being played.
    pub level: u8,
    /// The team everyone at that PC plays on (`ANY_TEAM`: the host
    /// chooses).
    pub team: u8,
}

struct Remote {
    conn: Connection,
    /// Where it joined from, for the log.
    address: String,
    computer: String,
    /// Who it is, if the online service says so: then that's who plays
    /// there, whatever its hello says.
    verified: Option<Verified>,
    /// The gamertag of the person at that PC, how they look, and the team
    /// each player there would like.
    name: String,
    look: Look,
    teams: Vec<u8>,
    since: Instant,
    /// Said hello and was let in (to the lobby or the game).
    welcomed: bool,
    /// Has players in the game being played.
    in_game: bool,
    /// Needs the whole game now (just joined).
    fresh: bool,
    /// Each player's latest controls, and buttons pressed since the last tick.
    players: Vec<(usize, Command, Command)>,
}

impl Remote {
    fn new(
        conn: Connection,
        address: String,
        computer: String,
        verified: Option<Verified>,
    ) -> Remote {
        Remote {
            conn,
            address,
            computer,
            verified,
            name: String::new(),
            look: Look::default(),
            teams: Vec::new(),
            since: Instant::now(),
            welcomed: false,
            in_game: false,
            fresh: false,
            players: Vec::new(),
        }
    }
}

pub struct Host {
    /// Where PCs join, unless they all come through `add_connection`.
    listener: Option<TcpListener>,
    port: u16,
    remotes: Vec<Remote>,
    beacon: Option<Beacon>,
    map: String,
    computer: String,
    /// The lobby, while there's no game on.
    lobby: Option<Lobby>,
    /// Joined PCs not heard from for this long are dropped.
    timeout: Duration,
}

/// A player's controls before their PC sends any: stand still, facing the
/// way they spawned.
fn seat(game: &Game, player: usize) -> (usize, Command, Command) {
    let still = Command {
        yaw: game.players[player].yaw,
        ..Command::default()
    };
    (player, still, Command::default())
}

/// What a joining PC said about itself.
struct Hello {
    /// The map the joining PC has loaded, and how many weapons and item
    /// spots it has there.
    map: String,
    weapons: usize,
    items: usize,
    computer: String,
    name: String,
    look: Look,
    teams: Vec<u8>,
}

impl Hello {
    /// Sent for this game: the same map, with the same contents.
    fn fits(&self, game: &Game, map: &str) -> bool {
        self.map.eq_ignore_ascii_case(map)
            && self.weapons == game.weapons.len()
            && self.items == game.item_spawns.len()
    }
}

/// Read a hello, or why it can't be accepted.
fn read_hello(r: &mut Reader) -> Result<Hello, String> {
    let bad = |_| "not a Halo 2 Rust game".to_string();
    if r.u32().map_err(bad)? != MAGIC {
        return Err("not a Halo 2 Rust game".into());
    }
    if r.u32().map_err(bad)? != PROTOCOL {
        return Err("DIFFERENT GAME VERSION, UPDATE BOTH PCS".into());
    }
    let map = r.str().map_err(bad)?;
    let weapons = r.u16().map_err(bad)? as usize;
    let items = r.u16().map_err(bad)? as usize;
    let locals = (r.u8().map_err(bad)? as usize).clamp(1, 4);
    Ok(Hello {
        map,
        weapons,
        items,
        computer: r.str().map_err(bad)?,
        name: r.str().map_err(bad)?,
        look: Look::read(r).map_err(bad)?,
        teams: (0..locals)
            .map(|_| r.u8())
            .collect::<Result<_, _>>()
            .map_err(bad)?,
    })
}

/// Listen at `address`. An IPv6 one takes IPv4 connections too where the
/// system allows it, and IPv6's any-address is IPv4's on systems without
/// IPv6.
fn listen(address: SocketAddr) -> std::io::Result<TcpListener> {
    let listener = match address {
        SocketAddr::V4(_) => TcpListener::bind(address)?,
        SocketAddr::V6(v6) => match listen_v6(address) {
            Err(_) if v6.ip().is_unspecified() => {
                TcpListener::bind((Ipv4Addr::UNSPECIFIED, address.port()))?
            }
            other => other?,
        },
    };
    listener.set_nonblocking(true)?;
    Ok(listener)
}

fn listen_v6(address: SocketAddr) -> std::io::Result<TcpListener> {
    let s = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))?;
    let _ = s.set_only_v6(false);
    // As the standard library does for its own listeners.
    #[cfg(unix)]
    s.set_reuse_address(true)?;
    s.bind(&address.into())?;
    s.listen(128)?;
    Ok(s.into())
}

impl Host {
    /// Listen for joining PCs on the first free port from `GAME_PORT`, and
    /// announce the game (on `map`) to the network.
    pub fn new(map: &str, session: u64) -> std::io::Result<Host> {
        let mut last = None;
        for port in crate::GAME_PORT..crate::GAME_PORT + 8 {
            let address = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
            match Host::bind(map, session, address, true) {
                Ok(host) => return Ok(host),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| std::io::Error::other("no free port")))
    }

    /// Listen for joining PCs at `address` (port 0: any free port), and
    /// announce the game to the network if `beacon`.
    pub fn bind(
        map: &str,
        session: u64,
        address: SocketAddr,
        beacon: bool,
    ) -> std::io::Result<Host> {
        let listener = listen(address)?;
        let port = listener.local_addr()?.port();
        let mut host = Host::online(map);
        host.listener = Some(listener);
        host.port = port;
        host.beacon = beacon.then(|| Beacon::new(session, port));
        Ok(host)
    }

    /// Host a game online: no listening and no announcing, PCs join only
    /// through `add_connection`.
    pub fn online(map: &str) -> Host {
        Host {
            listener: None,
            port: 0,
            remotes: Vec::new(),
            beacon: None,
            map: map.to_string(),
            computer: crate::computer_name(),
            lobby: None,
            timeout: crate::TIMEOUT,
        }
    }

    /// Drop joined PCs not heard from for this long (`TIMEOUT` unless set).
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// The port PCs join at (0 if they come through `add_connection`).
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Take a PC joining over `conn` (through the online service, say).
    /// It says hello as any other, but plays as who `verified` says: their
    /// gamertag, and everyone there on their team.
    pub fn add_connection(&mut self, conn: Connection, verified: Verified) {
        let address = format!("account {}", verified.account);
        let computer = verified.gamertag.clone();
        self.remotes
            .push(Remote::new(conn, address, computer, Some(verified)));
    }

    /// How many PCs have joined.
    pub fn joined(&self) -> usize {
        self.remotes.iter().filter(|r| r.welcomed).count()
    }

    /// The PCs that joined: the gamertag there, its look, and the team
    /// each player there would like (`ANY_TEAM`: any).
    pub fn members(&self) -> Vec<(String, Look, Vec<u8>)> {
        self.remotes
            .iter()
            .filter(|r| r.welcomed)
            .map(|r| (r.name.clone(), r.look, r.teams.clone()))
            .collect()
    }

    /// PCs from the lobby still loading the game's map (not in it yet).
    pub fn joining(&self) -> usize {
        self.remotes
            .iter()
            .filter(|r| r.welcomed && !r.in_game)
            .count()
    }

    /// Wait in the lobby (leaving any game: joined PCs' players are no
    /// longer theirs). Joined PCs are told whenever the lobby changes.
    pub fn set_lobby(&mut self, lobby: Lobby) {
        if self.lobby.is_none() {
            for r in &mut self.remotes {
                r.in_game = false;
                r.players.clear();
            }
        }
        if self.lobby.as_ref() == Some(&lobby) {
            return;
        }
        self.map = lobby.map.clone();
        let mut w = Writer::default();
        lobby.write(&mut w);
        for r in self.remotes.iter_mut().filter(|r| r.welcomed) {
            r.conn.send(kind::LOBBY, &w.0);
            let _ = r.conn.flush();
        }
        self.lobby = Some(lobby);
    }

    /// Start a game on `map`: joined PCs load it and say hello again.
    pub fn start(&mut self, map: &str) {
        self.lobby = None;
        self.map = map.to_string();
        let mut w = Writer::default();
        w.str(map);
        for r in self.remotes.iter_mut().filter(|r| r.welcomed) {
            r.in_game = false;
            r.players.clear();
            r.conn.send(kind::START, &w.0);
            let _ = r.conn.flush();
        }
    }

    /// Whether a joined PC controls this player.
    pub fn is_remote(&self, player: usize) -> bool {
        self.remotes
            .iter()
            .any(|r| r.players.iter().any(|p| p.0 == player))
    }

    /// Accept new PCs and read what joined PCs sent. New players are added
    /// to `game` here, up to `max_players` in all.
    pub fn poll(&mut self, game: &mut Game, max_players: usize) -> Vec<HostEvent> {
        let mut events = Vec::new();
        while let Some((stream, address)) = self.listener.as_ref().and_then(|l| l.accept().ok()) {
            match Connection::tcp(stream) {
                Ok(conn) => {
                    let computer = address.ip().to_string();
                    let remote = Remote::new(conn, address.to_string(), computer, None);
                    self.remotes.push(remote);
                }
                Err(e) => println!("lan: couldn't accept {address}: {e}"),
            }
        }
        let mut k = 0;
        while k < self.remotes.len() {
            match self.read_remote(k, game, max_players, &mut events) {
                Ok(()) => k += 1,
                Err(reason) => {
                    let r = self.remotes.remove(k);
                    println!("lan: {} ({}) left: {reason}", r.computer, r.address);
                    if r.welcomed {
                        events.push(HostEvent::Left {
                            computer: r.computer,
                            players: r.players.iter().map(|p| p.0).collect(),
                            reason,
                        });
                    }
                }
            }
        }
        let players = match &self.lobby {
            Some(l) => l.players.len(),
            None => game.players.len(),
        };
        if let Some(beacon) = &mut self.beacon {
            beacon.announce(&self.map, &self.computer, players.min(255) as u8);
        }
        events
    }

    fn read_remote(
        &mut self,
        k: usize,
        game: &mut Game,
        max_players: usize,
        events: &mut Vec<HostEvent>,
    ) -> Result<(), String> {
        let r = &mut self.remotes[k];
        if !r.welcomed && r.since.elapsed() > HELLO_TIMEOUT {
            return Err("never said hello".into());
        }
        for (kind, body) in r.conn.receive()? {
            let mut rd = Reader::new(&body);
            match kind {
                kind::HELLO if !r.in_game => {
                    // The lobby takes any map; a game, only its own.
                    let checked = read_hello(&mut rd).and_then(|h| {
                        if self.lobby.is_some() || h.fits(game, &self.map) {
                            Ok(Some(h))
                        } else if r.welcomed && !h.map.eq_ignore_ascii_case(&self.map) {
                            // Sent from the lobby as the game started.
                            Ok(None)
                        } else {
                            Err(format!("HOST IS PLAYING {}", self.map.to_uppercase()))
                        }
                    });
                    let hello = match checked {
                        Ok(Some(v)) => v,
                        Ok(None) => continue,
                        Err(why) => {
                            let mut w = Writer::default();
                            w.str(&why);
                            r.conn.send(kind::REFUSED, &w.0);
                            let _ = r.conn.flush();
                            return Err(why);
                        }
                    };
                    let Hello {
                        computer,
                        name,
                        look,
                        teams,
                        ..
                    } = hello;
                    let (name, teams) = match &r.verified {
                        Some(v) => (v.gamertag.clone(), vec![v.team; teams.len()]),
                        None => (name, teams),
                    };
                    let arriving = !r.welcomed;
                    r.computer = computer.clone();
                    r.name = name.clone();
                    r.look = look;
                    r.teams = teams.clone();
                    r.welcomed = true;
                    if let Some(lobby) = &self.lobby {
                        // Wait in the lobby for the next game.
                        let mut w = Writer::default();
                        lobby.write(&mut w);
                        r.conn.send(kind::LOBBY, &w.0);
                        if arriving {
                            println!("lan: {computer} ({}) is in the lobby", r.address);
                            events.push(HostEvent::Arrived { computer });
                        }
                        continue;
                    }
                    if game.players.len() + teams.len() > max_players {
                        let mut w = Writer::default();
                        w.str("THE GAME IS FULL");
                        r.conn.send(kind::REFUSED, &w.0);
                        let _ = r.conn.flush();
                        return Err("game full".into());
                    }
                    let players: Vec<usize> = teams
                        .iter()
                        .map(|&t| match t {
                            ANY_TEAM => game.add_player(),
                            t => game.add_player_on(t),
                        })
                        .collect();
                    for (k, &p) in players.iter().enumerate() {
                        game.set_name(p, &guest_name(&name, k));
                        game.set_look(p, look.guest(k));
                    }
                    let mut w = Writer::default();
                    w.str(&self.computer);
                    w.u8(players.len() as u8);
                    for &p in &players {
                        w.index(Some(p));
                    }
                    r.conn.send(kind::WELCOME, &w.0);
                    r.in_game = true;
                    r.fresh = true;
                    r.players = players.iter().map(|&p| seat(game, p)).collect();
                    println!("lan: {computer} ({}) joined", r.address);
                    events.push(HostEvent::Joined { computer, players });
                }
                kind::INPUT if r.in_game => {
                    let n = rd.u8().map_err(|e| e.to_string())?;
                    for _ in 0..n {
                        let player = rd.index().map_err(|e| e.to_string())?;
                        let cmd = Command::read(&mut rd).map_err(|e| e.to_string())?;
                        if let Some(slot) = r.players.iter_mut().find(|p| Some(p.0) == player) {
                            slot.1 = cmd;
                            slot.2 = slot.2.with_presses_from(&cmd);
                        }
                    }
                }
                kind::ADD_LOCAL if r.in_game => {
                    if game.players.len() < max_players && r.players.len() < 4 {
                        let p = game.add_player();
                        game.set_name(p, &guest_name(&r.name, r.players.len()));
                        game.set_look(p, r.look.guest(r.players.len()));
                        r.players.push(seat(game, p));
                        let mut w = Writer::default();
                        w.index(Some(p));
                        r.conn.send(kind::ADDED, &w.0);
                        events.push(HostEvent::Added { player: p });
                    }
                }
                kind::REMOVE_LOCAL if r.in_game => {
                    let player = rd.index().map_err(|e| e.to_string())?;
                    if r.players.len() > 1 {
                        if let Some(i) = r.players.iter().position(|p| Some(p.0) == player) {
                            let (p, ..) = r.players.remove(i);
                            events.push(HostEvent::Removed { player: p });
                        }
                    }
                }
                // Controls still on their way from a game that's over.
                kind::INPUT | kind::ADD_LOCAL | kind::REMOVE_LOCAL if r.welcomed => {}
                kind::ALIVE => {}
                _ => return Err(format!("unexpected message {kind}")),
            }
        }
        if r.conn.since_heard() > self.timeout {
            return Err("timed out".into());
        }
        r.conn.keep_alive(kind::HOST_ALIVE, self.timeout / 10);
        r.conn.flush()
    }

    /// Tell joined PCs we're still here, without reading what they sent:
    /// for while the game can't run (its window is minimized, say).
    pub fn keep_alive(&mut self) {
        for r in &mut self.remotes {
            r.conn.keep_alive(kind::HOST_ALIVE, self.timeout / 10);
            let _ = r.conn.flush();
        }
    }

    /// This tick's controls for a player on a joined PC: their latest
    /// controls plus any button they tapped since the last tick.
    pub fn command(&mut self, player: usize) -> Option<Command> {
        let slot = self
            .remotes
            .iter_mut()
            .flat_map(|r| r.players.iter_mut())
            .find(|p| p.0 == player)?;
        let cmd = slot.1.with_presses_from(&slot.2);
        slot.2 = Command::default();
        Some(cmd)
    }

    /// Send the game and what happened since the last call. `changed` is
    /// false when no tick ran (PCs that just joined still get the game).
    pub fn send(&mut self, game: &Game, events: &[Event], changed: bool) {
        if !self
            .remotes
            .iter()
            .any(|r| r.in_game && (changed || r.fresh))
        {
            return;
        }
        let mut w = Writer::default();
        game.write_state(&mut w);
        w.u16(events.len().min(u16::MAX as usize) as u16);
        for e in events.iter().take(u16::MAX as usize) {
            e.write(&mut w);
        }
        for r in &mut self.remotes {
            if r.in_game && (changed || r.fresh) {
                r.fresh = false;
                r.conn.send(kind::SNAPSHOT, &w.0);
                // A failure shows up as a departure on the next poll.
                let _ = r.conn.flush();
            }
        }
    }
}
