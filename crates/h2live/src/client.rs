//! A PC's side of the online service: signing in with the PC's own key,
//! answering pings, and keeping what the server last said (who we are, the
//! playlists, our party, who's online). The game and the tests both sign
//! in through this.
//!
//! In a match the server says what to do, and the game does it: load the
//! map (`LiveEvent::Match`); host it if asked (`HostMatch`, answered with
//! `ToServer::Hosting`); open a relay leg for each link it's given
//! (`Link`, then `LiveClient::open_leg`), which once joined is the
//! connection to the host (for `h2net::Client::over`) or to a joining PC
//! (for `h2net::Host::add_connection`); start (`Go`); and at the end send
//! how it went (`ToServer::Result`, see `results`), or that it left
//! (`ToServer::LeftMatch`). Then the levels come (`MatchOver`).

use crate::levels::{self, Finish};
use crate::store;
use ed25519_dalek::{Signer, SigningKey};
use h2net::live::{
    self, LinkInfo, Login, MatchInfo, MatchOver, OnlinePlayer, PartyInfo, PlayerResult,
    PlaylistInfo, SearchStatus, ToPc, ToServer, Welcome,
};
use h2net::Connection;
use h2sim::game::{clean_name, Look};
use h2sim::Game;
use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};

/// Notices and invites kept.
const NOTICES: usize = 10;
const INVITES: usize = 16;
/// Pings that can be on their way back at once.
const PINGS: usize = 16;

/// Who signs in, as the PC's profile has it.
#[derive(Debug, Clone, Default)]
pub struct Profile {
    pub gamertag: String,
    pub look: Look,
    /// The maps the PC has: file name (without `.map`) and
    /// `live::map_hash`.
    pub maps: Vec<(String, u64)>,
    /// Splitscreen guests playing.
    pub guests: u8,
}

/// What the server has said, as of the last poll.
#[derive(Debug, Clone, Default)]
pub struct View {
    /// Who we're signed in as, once we are, with our stat card.
    pub welcome: Option<Welcome>,
    /// The server's message of the day.
    pub motd: String,
    pub playlists: Vec<PlaylistInfo>,
    pub party: Option<PartyInfo>,
    /// Everyone signed in, us too.
    pub online: Vec<OnlinePlayer>,
    /// Parties we're invited to, and who asked us.
    pub invites: Vec<(u64, String)>,
    pub status: Option<SearchStatus>,
    /// The match we're in, from MATCH until MATCH_OVER.
    pub game: Option<MatchInfo>,
    /// The latest notices, oldest first.
    pub notices: Vec<String>,
    /// The latest round trip to the server, in milliseconds.
    pub round_trip: Option<u32>,
}

/// What happened, for the game to act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveEvent {
    /// Signed in, or the server sent our account again (after a rename).
    Welcomed,
    /// The server turned us away, and why (`live::GAMERTAG_TAKEN`, say).
    Refused(String),
    /// The connection to the server is gone.
    Lost(String),
    /// Asked into a party.
    Invited {
        party: u64,
        from: String,
    },
    Notice(String),
    /// A match is ready: load its map. It comes again, with another host,
    /// if the one asked didn't start hosting.
    Match(MatchInfo),
    /// Host this match: open `h2net::Host::online` on its map, then say so
    /// (`ToServer::Hosting`).
    HostMatch(u64),
    /// Open a relay leg (`LiveClient::open_leg`): to the host of a match
    /// or custom game, or as its host to a PC joining it.
    Link(LinkInfo),
    /// Start this match's game.
    Go(u64),
    /// A match is over, and what it did to our levels. (The new stat card
    /// on it is kept already.)
    MatchOver(MatchOver),
    /// Our party's leader opened a custom game on `map` (or moved it
    /// there): the leader hosts it, and everyone else is given a link.
    CustomOpen {
        party: u64,
        leader: u64,
        map: String,
    },
}

/// One end of a game relayed through the server: a connection to the
/// server's `/link`, which says which link it's for (LINK_HELLO) and waits
/// for the server to join it to the other end.
pub struct RelayLeg {
    /// The link, as the server gave it out (LINK).
    pub link: LinkInfo,
    conn: Option<Connection>,
}

impl RelayLeg {
    /// The connection to the other end, once the server joined the two
    /// legs (LINKED): what comes over it is the other end's. An error if
    /// the server gave up on the leg (no other end came) or the connection
    /// was taken already.
    pub fn poll(&mut self) -> Result<Option<Connection>, String> {
        let conn = self.conn.as_mut().ok_or("taken already")?;
        let mut messages = conn.receive()?;
        let Some((kind, body)) = messages.first() else {
            return Ok(None);
        };
        if ToPc::read(*kind, body) != Ok(ToPc::Linked) {
            return Err("not a relay leg".into());
        }
        messages.remove(0);
        let mut conn = self.conn.take().ok_or("taken already")?;
        // What came after LINKED is the game's.
        conn.put_back(messages);
        Ok(Some(conn))
    }
}

/// How a finished game ended, as a RESULT says: for each PC's account, the
/// Spartan of the person at it (`players`: account and player index), and
/// whether that PC quit (`left`). Places come from the game alone: in team
/// games the winning team's players first, then by team score; otherwise
/// by score, bots too. (The server puts those who quit last in
/// free-for-all games itself.)
pub fn results(game: &Game, players: &[(u64, usize)], left: &[u64]) -> Vec<PlayerResult> {
    let finishes: Vec<Finish> = game
        .players
        .iter()
        .map(|p| Finish {
            team: p.team,
            score: p.score,
            left: false,
        })
        .collect();
    let teams = game.rules.game_type.teams();
    let places = levels::places(&finishes, teams, game.winning_team);
    let count = |n: u32| n.min(u32::from(u16::MAX)) as u16;
    players
        .iter()
        .filter_map(|&(account, i)| {
            let p = game.players.get(i)?;
            Some(PlayerResult {
                account,
                team: p.team,
                place: places[i],
                score: p.score,
                kills: count(p.kills),
                deaths: count(p.deaths),
                left: left.contains(&account),
            })
        })
        .collect()
}

pub struct LiveClient {
    conn: Connection,
    key: SigningKey,
    /// Where our stat card is kept.
    card_path: PathBuf,
    pub view: View,
    /// When the server was last heard from.
    heard: f64,
    /// The next ping's number and when the last went, and pings on their
    /// way back (number, when sent).
    ping: u32,
    pinged: f64,
    pings: VecDeque<(u32, f64)>,
    gone: bool,
}

/// The key this PC signs in with, kept at `path` (identity.key, next to
/// the game's profile); made and saved there the first time.
pub fn identity(path: &Path) -> io::Result<SigningKey> {
    store::signing_key(path)
}

impl LiveClient {
    /// Sign in over `conn` as the holder of `key`, as `profile` says, at
    /// time `now` (in seconds, on a clock of the caller's). The stat card
    /// at `card_path` goes with it, if there is one, and the server's new
    /// cards are kept there.
    pub fn new(
        mut conn: Connection,
        key: SigningKey,
        profile: &Profile,
        card_path: &Path,
        now: f64,
    ) -> LiveClient {
        let card = std::fs::read_to_string(card_path).unwrap_or_default();
        let maps = profile
            .maps
            .iter()
            .filter(|(name, _)| name.len() <= live::MAX_NAME);
        let login = Login {
            key: key.verifying_key().to_bytes(),
            gamertag: clean_name(&profile.gamertag),
            look: profile.look,
            // One too big would only get us dropped.
            card: if card.len() <= live::MAX_CARD {
                card
            } else {
                String::new()
            },
            maps: maps.take(live::MAX_MAPS).cloned().collect(),
            guests: profile.guests.min(live::MAX_GUESTS),
        };
        ToServer::Login(login).send(&mut conn);
        // Errors surface as Lost on the first poll.
        let _ = conn.flush();
        LiveClient {
            conn,
            key,
            card_path: card_path.to_path_buf(),
            view: View::default(),
            heard: now,
            ping: 0,
            pinged: now,
            pings: VecDeque::new(),
            gone: false,
        }
    }

    /// Signed in, and still connected.
    pub fn signed_in(&self) -> bool {
        self.view.welcome.is_some() && !self.gone
    }

    /// Our account, once signed in.
    pub fn account(&self) -> Option<u64> {
        self.view.welcome.as_ref().map(|w| w.account)
    }

    /// Open a relay leg for `link` (from LINK) over `conn`, a new
    /// connection to the server's `/link`.
    pub fn open_leg(&self, link: LinkInfo, mut conn: Connection) -> RelayLeg {
        let hello = ToServer::LinkHello {
            token: link.token,
            account: self.account().unwrap_or(0),
        };
        hello.send(&mut conn);
        // Errors surface when polling the leg.
        let _ = conn.flush();
        RelayLeg {
            link,
            conn: Some(conn),
        }
    }

    /// Open a relay leg as `open_leg` does, over a connection `dial` makes.
    pub fn dial_leg(
        &self,
        link: LinkInfo,
        dial: impl FnOnce() -> Result<Connection, String>,
    ) -> Result<RelayLeg, String> {
        Ok(self.open_leg(link, dial()?))
    }

    /// Ask the server for something.
    pub fn send(&mut self, message: ToServer) {
        if let ToServer::Accept(p) | ToServer::Decline(p) | ToServer::JoinParty(p) = message {
            self.view.invites.retain(|&(party, _)| party != p);
        }
        message.send(&mut self.conn);
        // Errors surface as Lost on the next poll.
        let _ = self.conn.flush();
    }

    /// Take on everything the server sent, and ping it, at time `now`.
    pub fn poll(&mut self, now: f64) -> Vec<LiveEvent> {
        let mut events = Vec::new();
        if self.gone {
            return events;
        }
        let messages = match self.conn.receive() {
            Ok(m) => m,
            Err(why) => {
                self.gone = true;
                events.push(LiveEvent::Lost(why));
                return events;
            }
        };
        if !messages.is_empty() {
            self.heard = now;
        }
        for (kind, body) in messages {
            match ToPc::read(kind, &body) {
                Ok(message) => self.take(message, now, &mut events),
                Err(_) => {
                    self.gone = true;
                    events.push(LiveEvent::Lost("bad data from the server".into()));
                }
            }
            if self.gone {
                return events;
            }
        }
        if now - self.heard > live::TIMEOUT {
            self.gone = true;
            events.push(LiveEvent::Lost("the server stopped answering".into()));
            return events;
        }
        if now - self.pinged >= live::PING_EVERY {
            ToServer::Ping(self.ping).send(&mut self.conn);
            if self.pings.len() == PINGS {
                self.pings.pop_front();
            }
            self.pings.push_back((self.ping, now));
            self.ping = self.ping.wrapping_add(1);
            self.pinged = now;
        }
        let _ = self.conn.flush();
        events
    }

    fn take(&mut self, message: ToPc, now: f64, events: &mut Vec<LiveEvent>) {
        let view = &mut self.view;
        match message {
            ToPc::Challenge { nonce, motd } => {
                let proof = live::proof(&nonce, self.key.verifying_key().as_bytes());
                ToServer::Prove(self.key.sign(&proof).to_bytes()).send(&mut self.conn);
                view.motd = motd;
            }
            ToPc::Welcome(welcome) => {
                self.keep_card(&welcome.card);
                self.view.welcome = Some(welcome);
                events.push(LiveEvent::Welcomed);
            }
            ToPc::Refused(why) => {
                self.gone = true;
                events.push(LiveEvent::Refused(why));
            }
            ToPc::Playlists(playlists) => view.playlists = playlists,
            ToPc::Online(online) => view.online = online,
            ToPc::Party(party) => {
                view.invites.retain(|&(id, _)| id != party.id);
                view.party = Some(party);
            }
            ToPc::Invited { party, from } => {
                view.invites.retain(|&(id, _)| id != party);
                if view.invites.len() == INVITES {
                    view.invites.remove(0);
                }
                view.invites.push((party, from.clone()));
                events.push(LiveEvent::Invited { party, from });
            }
            ToPc::Status(status) => view.status = Some(status),
            ToPc::Notice(text) => {
                if view.notices.len() == NOTICES {
                    view.notices.remove(0);
                }
                view.notices.push(text.clone());
                events.push(LiveEvent::Notice(text));
            }
            ToPc::Ping(n) => ToServer::Pong(n).send(&mut self.conn),
            ToPc::Pong(n) => {
                if let Some(i) = self.pings.iter().position(|&(p, _)| p == n) {
                    let sent = self.pings[i].1;
                    self.pings.drain(..=i);
                    view.round_trip = Some(((now - sent) * 1000.0).round() as u32);
                }
            }
            ToPc::Match(info) => {
                view.game = Some(info.clone());
                events.push(LiveEvent::Match(info));
            }
            ToPc::HostMatch(id) => events.push(LiveEvent::HostMatch(id)),
            ToPc::Link(link) => events.push(LiveEvent::Link(link)),
            ToPc::Go(id) => events.push(LiveEvent::Go(id)),
            ToPc::MatchOver(over) => {
                if view.game.as_ref().is_some_and(|g| g.id == over.id) {
                    view.game = None;
                }
                self.keep_card(&over.card);
                events.push(LiveEvent::MatchOver(over));
            }
            ToPc::CustomOpen { party, leader, map } => {
                events.push(LiveEvent::CustomOpen { party, leader, map });
            }
            // Only relay legs are told they're linked.
            ToPc::Linked => {}
        }
    }

    /// Keep the stat card the server sent, for next time.
    fn keep_card(&self, card: &str) {
        if card.is_empty() {
            return;
        }
        if let Err(e) = store::replace(&self.card_path, card) {
            println!(
                "live: can't keep the stat card in {}: {e}",
                self.card_path.display()
            );
        }
    }
}
