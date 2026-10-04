//! Hosting: accept PCs that join, give each of their players a Spartan, run
//! their controls through the game and send everyone the result.

use crate::conn::Connection;
use crate::discovery::Beacon;
use crate::{kind, MAGIC, PROTOCOL};
use h2sim::game::{guest_name, Event, Look, Reader, Writer};
use h2sim::{Command, Game};
use std::net::{SocketAddr, TcpListener};
use std::time::{Duration, Instant};

/// Joining PCs must say hello this soon.
const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// Things the host's game should show or act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    /// A PC joined with these players.
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

struct Remote {
    conn: Connection,
    address: SocketAddr,
    computer: String,
    /// The gamertag of the person at that PC, and how they look.
    name: String,
    look: Look,
    since: Instant,
    welcomed: bool,
    /// Needs the whole game now (just joined).
    fresh: bool,
    /// Each player's latest controls, and buttons pressed since the last tick.
    players: Vec<(usize, Command, Command)>,
}

pub struct Host {
    listener: TcpListener,
    port: u16,
    remotes: Vec<Remote>,
    beacon: Beacon,
    map: String,
    computer: String,
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
    computer: String,
    name: String,
    look: Look,
    locals: usize,
}

/// Why a hello can't be accepted.
fn check_hello(r: &mut Reader, game: &Game, map: &str) -> Result<Hello, String> {
    let bad = |_| "not a Halo 2 Rust game".to_string();
    if r.u32().map_err(bad)? != MAGIC {
        return Err("not a Halo 2 Rust game".into());
    }
    if r.u32().map_err(bad)? != PROTOCOL {
        return Err("DIFFERENT GAME VERSION, UPDATE BOTH PCS".into());
    }
    let their_map = r.str().map_err(bad)?;
    let weapons = r.u16().map_err(bad)? as usize;
    let items = r.u16().map_err(bad)? as usize;
    let locals = (r.u8().map_err(bad)? as usize).max(1);
    let computer = r.str().map_err(bad)?;
    let name = r.str().map_err(bad)?;
    let look = Look::read(r).map_err(bad)?;
    if !their_map.eq_ignore_ascii_case(map)
        || weapons != game.weapons.len()
        || items != game.item_spawns.len()
    {
        return Err(format!("HOST IS PLAYING {}", map.to_uppercase()));
    }
    Ok(Hello {
        computer,
        name,
        look,
        locals,
    })
}

impl Host {
    /// Listen for joining PCs on the first free port from `GAME_PORT`, and
    /// announce the game (on `map`) to the network.
    pub fn new(map: &str, session: u64) -> std::io::Result<Host> {
        let mut last = None;
        for port in crate::GAME_PORT..crate::GAME_PORT + 8 {
            match TcpListener::bind(("0.0.0.0", port)) {
                Ok(listener) => {
                    listener.set_nonblocking(true)?;
                    let computer = crate::computer_name();
                    return Ok(Host {
                        listener,
                        port,
                        remotes: Vec::new(),
                        beacon: Beacon::new(session, port),
                        map: map.to_string(),
                        computer,
                    });
                }
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| std::io::Error::other("no free port")))
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// How many PCs have joined.
    pub fn joined(&self) -> usize {
        self.remotes.iter().filter(|r| r.welcomed).count()
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
        while let Ok((stream, address)) = self.listener.accept() {
            match Connection::new(stream) {
                Ok(conn) => self.remotes.push(Remote {
                    conn,
                    address,
                    computer: address.ip().to_string(),
                    name: String::new(),
                    look: Look::default(),
                    since: Instant::now(),
                    welcomed: false,
                    fresh: false,
                    players: Vec::new(),
                }),
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
        let players = game.players.len().min(255) as u8;
        self.beacon.announce(&self.map, &self.computer, players);
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
                kind::HELLO if !r.welcomed => {
                    let hello = match check_hello(&mut rd, game, &self.map) {
                        Ok(v) => v,
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
                        locals,
                    } = hello;
                    if game.players.len() + locals > max_players {
                        let mut w = Writer::default();
                        w.str("THE GAME IS FULL");
                        r.conn.send(kind::REFUSED, &w.0);
                        let _ = r.conn.flush();
                        return Err("game full".into());
                    }
                    let players: Vec<usize> = (0..locals).map(|_| game.add_player()).collect();
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
                    r.computer = computer.clone();
                    r.name = name;
                    r.look = look;
                    r.welcomed = true;
                    r.fresh = true;
                    r.players = players.iter().map(|&p| seat(game, p)).collect();
                    println!("lan: {computer} ({}) joined", r.address);
                    events.push(HostEvent::Joined { computer, players });
                }
                kind::INPUT if r.welcomed => {
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
                kind::ADD_LOCAL if r.welcomed => {
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
                kind::REMOVE_LOCAL if r.welcomed => {
                    let player = rd.index().map_err(|e| e.to_string())?;
                    if r.players.len() > 1 {
                        if let Some(i) = r.players.iter().position(|p| Some(p.0) == player) {
                            let (p, ..) = r.players.remove(i);
                            events.push(HostEvent::Removed { player: p });
                        }
                    }
                }
                _ => return Err(format!("unexpected message {kind}")),
            }
        }
        r.conn.flush()
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
            .any(|r| r.welcomed && (changed || r.fresh))
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
            if r.welcomed && (changed || r.fresh) {
                r.fresh = false;
                r.conn.send(kind::SNAPSHOT, &w.0);
                // A failure shows up as a departure on the next poll.
                let _ = r.conn.flush();
            }
        }
    }
}
