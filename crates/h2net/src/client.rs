//! Joining another PC's game: send our players' controls, show what the
//! host sends back.

use crate::conn::Connection;
use crate::{kind, MAGIC, PROTOCOL};
use h2sim::game::{Event, Look, Reader, Writer};
use h2sim::{Command, Game};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

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
}

pub struct Client {
    conn: Connection,
    pub address: SocketAddr,
    /// Our players in the host's game, once welcomed.
    pub players: Vec<usize>,
    /// Game states received so far.
    pub snapshots: u64,
    gone: bool,
}

impl Client {
    /// Connect to a host and ask to play with `locals` players, going by
    /// `name` and `look`. `game` must be the same map the host is on.
    pub fn connect(
        address: SocketAddr,
        game: &Game,
        map: &str,
        locals: usize,
        (name, look): (&str, Look),
    ) -> std::io::Result<Client> {
        let stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)?;
        let mut conn = Connection::new(stream)?;
        let mut w = Writer::default();
        w.u32(MAGIC);
        w.u32(PROTOCOL);
        w.str(map);
        w.u16(game.weapons.len() as u16);
        w.u16(game.item_spawns.len() as u16);
        w.u8(locals.clamp(1, 4) as u8);
        w.str(&crate::computer_name());
        w.str(name);
        look.write(&mut w);
        conn.send(kind::HELLO, &w.0);
        conn.flush().map_err(std::io::Error::other)?;
        Ok(Client {
            conn,
            address,
            players: Vec::new(),
            snapshots: 0,
            gone: false,
        })
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
        let messages = match self.conn.receive() {
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
                kind::SNAPSHOT => (|| {
                    game.read_state(&mut r)?;
                    self.snapshots += 1;
                    let n = r.u16()?;
                    for _ in 0..n {
                        let (p, w, v) =
                            (game.players.len(), game.weapons.len(), game.vehicles.len());
                        events.push(Event::read(&mut r, p, w, v)?);
                    }
                    Ok(())
                })(),
                _ => Err(h2sim::game::Malformed),
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
        (out, events)
    }

    /// Our players' controls for this frame.
    pub fn send_commands(&mut self, commands: &[(usize, Command)]) {
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
