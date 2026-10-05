//! Joining another PC's game: send our players' controls, show what the
//! host sends back.

use crate::conn::Connection;
use crate::{after, kind, Lobby, Pace, ANY_TEAM, MAGIC, PROTOCOL};
use h2sim::game::{Event, Look, Malformed, Reader, Writer, TICK};
use h2sim::{Command, Game};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// Most snapshots kept that came early: more than a host ever sends past the
/// newest a PC says it has taken on.
const EARLY: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientEvent {
    /// The host took us in; these are our players.
    Welcomed {
        /// The host PC's name.
        computer: String,
        players: Vec<usize>,
    },
    /// The host said no, and why.
    Refused(String),
    /// The host added a splitscreen player for us.
    Added(usize),
    /// The connection to the host is gone.
    Lost(String),
    /// The host is in its lobby (or changed it): wait there. Our players
    /// are gone with the game.
    Lobby(Lobby),
    /// The host started a game on this map: load it, then `rejoin`.
    Start(String),
}

pub struct Client {
    conn: Connection,
    /// Our players in the host's game, once welcomed.
    pub players: Vec<usize>,
    /// Game states received so far.
    pub snapshots: u64,
    /// In the host's game (rather than its lobby).
    pub in_game: bool,
    /// The gamertag and look we join with.
    me: (String, Look),
    gone: bool,
    /// The host is gone if not heard from for this long.
    timeout: Duration,
    /// The controls we last sent; they go once a tick.
    sent: Vec<(usize, Command)>,
    input: Pace,
    /// The last snapshot, and its number: the next may come as how it
    /// differs from this one.
    last: Option<(u32, Vec<u8>)>,
    /// Snapshots sent as how they differ from one that hasn't come yet,
    /// by number. Through the online service's relay those go to many PCs
    /// at once (see `Host::set_fanout`), and can overtake a whole snapshot
    /// sent to this PC alone.
    early: Vec<(u32, Vec<u8>)>,
    /// The number of the newest snapshot taken on, and the last the host
    /// was told of: what it sent since is on its way, or came early.
    got: u32,
    told: u32,
}

/// A command's buttons, without where it aims or moves.
fn buttons(c: &Command) -> Command {
    Command {
        movement: Default::default(),
        yaw: 0.0,
        pitch: 0.0,
        rise: 0.0,
        ..*c
    }
}

/// What we say to the host to join its game, or its lobby: the map we have
/// loaded, and who plays here (with the team each would like).
fn hello(game: &Game, map: &str, teams: &[u8], (name, look): (&str, Look)) -> Writer {
    let locals = teams.len().clamp(1, 4);
    let mut w = Writer::default();
    w.u32(MAGIC);
    w.u32(PROTOCOL);
    w.str(map);
    w.u16(game.weapons.len() as u16);
    w.u16(game.item_spawns.len() as u16);
    w.u8(locals as u8);
    w.str(&crate::computer_name());
    w.str(name);
    look.write(&mut w);
    for k in 0..locals {
        w.u8(teams.get(k).copied().unwrap_or(ANY_TEAM));
    }
    w
}

impl Client {
    /// Connect to a host and ask to play with a player for each of `teams`
    /// (`ANY_TEAM` lets the host choose), going by
    /// `name` and `look`. `game` must be the same map the host is on.
    pub fn connect(
        address: SocketAddr,
        game: &Game,
        map: &str,
        teams: &[u8],
        me: (&str, Look),
    ) -> std::io::Result<Client> {
        let stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)?;
        let mut client = Client::over(Connection::tcp(stream)?, game, map, teams, me);
        client.conn.flush().map_err(std::io::Error::other)?;
        Ok(client)
    }

    /// Join a host over `conn` (through the online service, say), as
    /// `connect` does.
    pub fn over(
        mut conn: Connection,
        game: &Game,
        map: &str,
        teams: &[u8],
        me: (&str, Look),
    ) -> Client {
        conn.send(kind::HELLO, &hello(game, map, teams, me).0);
        // Errors surface as Lost on the first poll.
        let _ = conn.flush();
        Client {
            conn,
            players: Vec::new(),
            snapshots: 0,
            in_game: false,
            me: (me.0.to_string(), me.1),
            gone: false,
            timeout: crate::TIMEOUT,
            sent: Vec::new(),
            input: Pace::new(TICK),
            last: None,
            early: Vec::new(),
            got: 0,
            told: 0,
        }
    }

    /// Give up on the host if not heard from for this long (`TIMEOUT`
    /// unless set).
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// Join the game the host started (`ClientEvent::Start`), once `game`
    /// is its map. In the lobby, tells the host who plays here now.
    pub fn rejoin(&mut self, game: &Game, map: &str, teams: &[u8]) {
        let me = (self.me.0.as_str(), self.me.1);
        let w = hello(game, map, teams, me);
        self.conn.send(kind::HELLO, &w.0);
        self.flush();
    }

    /// Take on everything the host sent. Returns what happened to the
    /// connection, and the game's events since the last call (to show as if
    /// they happened here).
    pub fn poll(&mut self, game: &mut Game) -> (Vec<ClientEvent>, Vec<Event>) {
        let mut out = Vec::new();
        let mut events = Vec::new();
        if self.gone {
            return (out, events);
        }
        let received = self.conn.receive().and_then(|m| {
            if self.conn.since_heard() > self.timeout {
                return Err("timed out".to_string());
            }
            Ok(m)
        });
        let messages = match received {
            Ok(m) => m,
            Err(why) => {
                self.gone = true;
                out.push(ClientEvent::Lost(why));
                return (out, events);
            }
        };
        for (kind, body) in messages {
            let mut r = Reader::new(&body);
            let result = match kind {
                kind::WELCOME => (|| {
                    let computer = r.str()?;
                    let n = r.u8()?;
                    let players = (0..n)
                        .map(|_| r.index_below(256))
                        .collect::<Result<Vec<_>, _>>()?;
                    self.players = players.clone();
                    self.in_game = true;
                    // (What came early stays: the whole game it follows
                    // comes next.)
                    self.last = None;
                    out.push(ClientEvent::Welcomed { computer, players });
                    Ok(())
                })(),
                kind::REFUSED => r.str().map(|why| {
                    self.gone = true;
                    out.push(ClientEvent::Refused(why));
                }),
                kind::ADDED => r.index_below(256).map(|p| {
                    self.players.push(p);
                    out.push(ClientEvent::Added(p));
                }),
                kind::LOBBY => Lobby::read(&mut r).map(|lobby| {
                    self.in_game = false;
                    self.players.clear();
                    self.last = None;
                    self.early.clear();
                    out.push(ClientEvent::Lobby(lobby));
                }),
                kind::START => r.str().map(|map| {
                    self.in_game = false;
                    self.players.clear();
                    self.last = None;
                    self.early.clear();
                    out.push(ClientEvent::Start(map));
                }),
                kind::HOST_ALIVE => Ok(()),
                // The last of a game that's over.
                kind::SNAPSHOT if !self.in_game => Ok(()),
                kind::SNAPSHOT => self
                    .take_snapshot(game, body, &mut events)
                    .and_then(|()| self.take_early(game, &mut events)),
                kind::SNAPSHOT_DELTA => self.take_delta(game, body, &mut events),
                _ => Err(Malformed),
            };
            if result.is_err() {
                self.gone = true;
                out.push(ClientEvent::Lost("bad data from the host".into()));
                break;
            }
            if self.gone {
                break;
            }
        }
        // So the host knows how many are still on their way.
        if self.got != self.told && !self.gone {
            let mut w = Writer::default();
            w.u32(self.got);
            self.conn.send(kind::GOT, &w.0);
            self.told = self.got;
        }
        self.keep_alive();
        (out, events)
    }

    /// Tell the host we're still here, without reading what it sent: for
    /// while the game can't run (its window is minimized, say).
    pub fn keep_alive(&mut self) {
        if !self.gone {
            self.conn.keep_alive(kind::ALIVE, self.timeout / 10);
            self.flush();
        }
    }

    /// Take on a snapshot: the game, and what happened since the last.
    fn take_snapshot(
        &mut self,
        game: &mut Game,
        snapshot: Vec<u8>,
        events: &mut Vec<Event>,
    ) -> Result<(), Malformed> {
        let mut r = Reader::new(&snapshot);
        let seq = r.u32()?;
        game.read_state(&mut r)?;
        self.snapshots += 1;
        let n = r.u16()?;
        for _ in 0..n {
            let (p, w, v) = (game.players.len(), game.weapons.len(), game.vehicles.len());
            events.push(Event::read(&mut r, p, w, v)?);
        }
        self.last = Some((seq, snapshot));
        self.got = seq;
        Ok(())
    }

    /// Take on a snapshot sent as how it differs from the one before it,
    /// and those that came early and follow it; or keep it, if the one
    /// before hasn't come yet. One no newer than the last taken on is from
    /// before that (the game before, say), and is dropped, as is one more
    /// than can be kept.
    fn take_delta(
        &mut self,
        game: &mut Game,
        message: Vec<u8>,
        events: &mut Vec<Event>,
    ) -> Result<(), Malformed> {
        let seq = Reader::new(&message).u32()?;
        match self.last.as_ref().map(|l| l.0) {
            Some(last) if !after(seq, last) => Ok(()),
            Some(last) if seq == last.wrapping_add(1) => {
                let snapshot = self.undelta(&message).ok_or(Malformed)?;
                self.take_snapshot(game, snapshot, events)?;
                self.take_early(game, events)
            }
            _ if self.early.len() < EARLY => {
                self.early.push((seq, message));
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Take on the snapshots that came early and now follow the last one,
    /// dropping those from before it.
    fn take_early(&mut self, game: &mut Game, events: &mut Vec<Event>) -> Result<(), Malformed> {
        while let Some(last) = self.last.as_ref().map(|l| l.0) {
            self.early.retain(|(seq, _)| after(*seq, last));
            let next = last.wrapping_add(1);
            let Some(k) = self.early.iter().position(|(seq, _)| *seq == next) else {
                break;
            };
            let (_, message) = self.early.swap_remove(k);
            let snapshot = self.undelta(&message).ok_or(Malformed)?;
            self.take_snapshot(game, snapshot, events)?;
        }
        Ok(())
    }

    /// A snapshot sent as how it differs from the last one.
    fn undelta(&self, message: &[u8]) -> Option<Vec<u8>> {
        let mut r = Reader::new(message);
        let (seq, len) = (r.u32().ok()?, r.u32().ok()? as usize);
        let (last_seq, last) = self.last.as_ref()?;
        if seq != last_seq.wrapping_add(1) {
            return None;
        }
        crate::delta::decode(last, &message[8..], len)
    }

    /// Our players' controls for this frame. They go to the host once a
    /// tick, as it runs the game, except that a button pressed or let go
    /// goes at once (so taps between ticks are not lost).
    pub fn send_commands(&mut self, commands: &[(usize, Command)]) {
        let pressed = commands.len() != self.sent.len()
            || commands
                .iter()
                .zip(&self.sent)
                .any(|(a, b)| a.0 != b.0 || buttons(&a.1) != buttons(&b.1));
        if !self.input.due() && !pressed {
            return;
        }
        self.input.went();
        self.sent = commands.to_vec();
        let mut w = Writer::default();
        w.u8(commands.len().min(255) as u8);
        for (p, c) in commands.iter().take(255) {
            w.index(Some(*p));
            c.write(&mut w);
        }
        self.conn.send(kind::INPUT, &w.0);
        self.flush();
    }

    /// Ask for another splitscreen player.
    pub fn add_local(&mut self) {
        self.conn.send(kind::ADD_LOCAL, &[]);
        self.flush();
    }

    /// One of our splitscreen players stops playing.
    pub fn remove_local(&mut self, player: usize) {
        let mut w = Writer::default();
        w.index(Some(player));
        self.conn.send(kind::REMOVE_LOCAL, &w.0);
        self.players.retain(|&p| p != player);
        self.flush();
    }

    fn flush(&mut self) {
        // Errors surface as Lost on the next poll.
        let _ = self.conn.flush();
    }
}
