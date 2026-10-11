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
//!
//! The launcher (h2launch) signs in through this too, without the game's
//! renderer or engine: `LiveClient::launcher`, then parties and searches
//! as the game does (`send`), from its own playlists. A match comes as
//! `LiveEvent::LauncherMatch`: join its room on the relay (`join_relay`,
//! at the address the server was dialled at, `LiveClient::server_ip`) and
//! start halo2.dll's online game on its map and variant, as host if it
//! names this PC's relay id as the host (then say `ToServer::Hosting`
//! once hosting, after `HostMatch`), or else joining the host (then say
//! `ToServer::Joined`). `Go` starts it; at the end send the engine's
//! results (`ToServer::LauncherResult`, everyone, the host's counting),
//! or that it left (`ToServer::LeftMatch`). Then the levels come
//! (`MatchOver`).
//!
//! A launcher also has a friends list, which the server sends whole when
//! it changes (at most once a second; `View::friends`, and
//! `LiveEvent::Friends`), and asks for players' service records
//! (`ToServer::Record`, answered with `LiveEvent::ServiceRecord`). The game
//! is never sent either.
//!
//! On a server whose owner makes the accounts, a launcher on a PC the
//! account doesn't know yet signs in with the account's password
//! (`set_access`, before the first poll). The server first proves who it is
//! (SEAL_KEY: a key made for this connection, signed with the server's
//! identity key, which the launcher may pin), and the password goes sealed
//! to that key (`crate::seal`), never in clear. From then on the PC signs
//! in by its key alone. The accounts it may see come as
//! `LiveEvent::Accounts` (in `View::accounts`), and an admin changes them
//! with `ask_account`.

use crate::levels::{self, Finish};
use crate::password::{self, Secret};
use crate::seal;
use crate::store;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use h2net::live::{
    self, AccountOp, Accounts, Friend, LauncherMatch, LinkInfo, Login, LoginClient, MatchInfo,
    MatchOver, OnlinePlayer, PartyInfo, PlayerResult, PlaylistInfo, Sealed, SearchStatus,
    ServiceRecord, ToPc, ToServer, Welcome,
};
use h2net::Connection;
use h2relay::{ClientConfig, MemberKey, RelayClient};
use h2sim::game::{clean_name, Look};
use h2sim::Game;
use std::collections::VecDeque;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

/// Notices and invites kept.
const NOTICES: usize = 10;
const INVITES: usize = 16;
/// Pings that can be on their way back at once.
const PINGS: usize = 16;
/// Why a launcher gave up on a server that didn't sign its SEAL_KEY.
pub const SERVER_UNPROVEN: &str = "THE SERVER DIDN'T PROVE WHO IT IS";

/// How a launcher signs in (`LiveClient::set_access`). The default: any
/// server, and no password (the PC signs in by its key alone). Its Debug
/// doesn't show the password.
#[derive(Debug, Clone, Default)]
pub struct Access {
    /// The identity key the server must have (the one it had last time);
    /// with another, the launcher gives up (`LiveEvent::KeyChanged`).
    pub pin: Option<[u8; 32]>,
    /// The account's password, for a PC the account doesn't know yet.
    pub password: Option<Secret>,
    /// This PC's name, as the account's list of PCs shows it.
    pub computer: String,
}

/// What a launcher asks of the accounts (`LiveClient::ask_account`). Every
/// ask but `ChangePassword` is an admin's. (Its Debug doesn't show
/// passwords.)
#[derive(Debug, Clone)]
pub enum AccountAsk {
    /// Send the accounts again.
    List,
    Create {
        gamertag: String,
        password: Secret,
    },
    SetPassword {
        account: u64,
        password: Secret,
    },
    SignOut {
        account: u64,
    },
    Remove {
        account: u64,
    },
    /// Our own account's.
    ChangePassword {
        old: Secret,
        new: Secret,
    },
}

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
    /// A launcher's: the match it's in, from LAUNCHER_MATCH until
    /// MATCH_OVER.
    pub launcher_game: Option<LauncherMatch>,
    /// The latest notices, oldest first.
    pub notices: Vec<String>,
    /// The latest round trip to the server, in milliseconds.
    pub round_trip: Option<u32>,
    /// A launcher's friends list, as the server last sent it: requests to
    /// us (oldest first), friends (by gamertag), then requests we sent.
    pub friends: Vec<Friend>,
    /// A launcher's: the accounts it may see, as the server last sent them.
    pub accounts: Option<Accounts>,
    /// A launcher's: the server's identity key, once it proved it holds it
    /// (to pin next time).
    pub server_key: Option<[u8; 32]>,
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
    /// A launcher's match is ready: join its room on the relay
    /// (`join_relay`) and host it or join its host. It comes again, with
    /// another host, if the one asked didn't start hosting.
    LauncherMatch(LauncherMatch),
    /// A launcher's friends list came (it's in `View::friends`).
    Friends,
    /// A player's service record, as asked for (`ToServer::Record`). The
    /// view doesn't keep them.
    ServiceRecord(ServiceRecord),
    /// A launcher's accounts came (they're in `View::accounts`, as last
    /// sent), maybe answering an `ask_account`: `answers` its number (0
    /// for none), whether it was done, and what to show. (Each message's
    /// own, so an answer isn't lost under a list that came after it in
    /// the same poll.)
    Accounts {
        answers: u32,
        ok: bool,
        text: String,
    },
    /// The server's identity key isn't the one pinned (`Access::pin`), so
    /// the launcher gave up before sending anything after its LOGIN (no
    /// password, no proof of its key): a new server, or someone in between.
    KeyChanged {
        was: [u8; 32],
        now: [u8; 32],
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

/// Where a launcher's match is on the relay: the relay's port at `server`,
/// the address this PC dialled the server at (`LiveClient::server_ip`).
pub fn relay_addr(game: &LauncherMatch, server: IpAddr) -> SocketAddr {
    SocketAddr::new(server, game.relay.port)
}

/// The key a launcher's match gave this PC for its id in the match's room.
pub fn member_key(game: &LauncherMatch) -> MemberKey {
    MemberKey(game.relay.key)
}

/// Join a launcher's match's room on the relay at `server` (see
/// `relay_addr`) as this PC's relay id, with `config` and the key the match
/// gave. The engine's packets then go through the client returned.
pub fn join_relay(
    game: &LauncherMatch,
    server: IpAddr,
    config: ClientConfig,
) -> io::Result<RelayClient> {
    let config = ClientConfig {
        member_key: Some(member_key(game)),
        ..config
    };
    RelayClient::connect_with(
        relay_addr(game, server),
        game.relay.room,
        game.relay.id,
        config,
    )
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
    /// A launcher, not the game.
    launcher: bool,
    /// The gamertag our LOGIN named, as sent.
    gamertag: String,
    access: Access,
    /// The SEAL_KEY that came before the challenge (key, server, sig).
    seal_key: Option<([u8; 32], [u8; 32], [u8; 64])>,
    /// Once the server proved it holds its identity key: the key to seal
    /// passwords to on this connection, and the challenge.
    seal: Option<([u8; 32], [u8; 32])>,
    /// The number of our last account request.
    account_seq: u32,
    /// The server's challenge came. A server sends one SEAL_KEY and one
    /// CHALLENGE on a connection, so another of either after it is someone
    /// else's (one in between, offering their own key to seal to).
    challenged: bool,
}

/// The key this PC signs in with, kept at `path` (identity.key, next to
/// the game's profile); made and saved there the first time.
pub fn identity(path: &Path) -> io::Result<SigningKey> {
    store::signing_key(path)
}

impl LiveClient {
    /// Sign the game in over `conn` as the holder of `key`, as `profile`
    /// says, at time `now` (in seconds, on a clock of the caller's). The
    /// stat card at `card_path` goes with it, if there is one, and the
    /// server's new cards are kept there.
    pub fn new(
        conn: Connection,
        key: SigningKey,
        profile: &Profile,
        card_path: &Path,
        now: f64,
    ) -> LiveClient {
        let client = LoginClient::viewer();
        LiveClient::sign_in(conn, key, profile, client, card_path, now)
    }

    /// Sign a launcher in, as `new` signs in the game, saying the
    /// launcher's `version` (for the server's log). Its profile's maps are
    /// MCC's, each with `live::map_hash` of its file.
    pub fn launcher(
        conn: Connection,
        key: SigningKey,
        profile: &Profile,
        version: &str,
        card_path: &Path,
        now: f64,
    ) -> LiveClient {
        // One too long would only get us dropped.
        let mut version = version.to_string();
        while version.len() > live::MAX_NAME {
            version.pop();
        }
        let client = LoginClient::Launcher { version };
        LiveClient::sign_in(conn, key, profile, client, card_path, now)
    }

    fn sign_in(
        mut conn: Connection,
        key: SigningKey,
        profile: &Profile,
        client: LoginClient,
        card_path: &Path,
        now: f64,
    ) -> LiveClient {
        let card = std::fs::read_to_string(card_path).unwrap_or_default();
        let launcher = client.kind() == live::ClientKind::Launcher;
        let maps = profile
            .maps
            .iter()
            .filter(|(name, _)| name.len() <= live::MAX_NAME);
        let login = Login {
            client,
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
        let gamertag = login.gamertag.clone();
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
            launcher,
            gamertag,
            access: Access::default(),
            seal_key: None,
            seal: None,
            account_seq: 0,
            challenged: false,
        }
    }

    /// How a launcher signs in: the server key it must have, and the
    /// password for a PC the account doesn't know yet. Set it right after
    /// `launcher`, before the first poll. (The game has no say.)
    pub fn set_access(&mut self, access: Access) {
        self.access = access;
    }

    /// Signed in, and still connected.
    pub fn signed_in(&self) -> bool {
        self.view.welcome.is_some() && !self.gone
    }

    /// Our account, once signed in.
    pub fn account(&self) -> Option<u64> {
        self.view.welcome.as_ref().map(|w| w.account)
    }

    /// The address the server was dialled at (as the system found it), to
    /// reach the relay beside it (`relay_addr`). None over a connection
    /// within this process (a test's).
    pub fn server_ip(&self) -> Option<IpAddr> {
        let stream = self.conn.stream()?;
        Some(stream.peer_addr().ok()?.ip().to_canonical())
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
    pub fn send(&mut self, mut message: ToServer) {
        match &mut message {
            ToServer::Accept(p) | ToServer::Decline(p) | ToServer::JoinParty(p) => {
                self.view.invites.retain(|&(party, _)| party != *p);
            }
            ToServer::Profile { gamertag, .. } => *gamertag = clean_name(gamertag),
            _ => {}
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
        // One SEAL_KEY, then one CHALLENGE, per connection: anything more
        // is bad data, and nothing is answered or sealed to it.
        let again = match message {
            ToPc::Challenge { .. } => std::mem::replace(&mut self.challenged, true),
            ToPc::SealKey { .. } => self.challenged || self.seal_key.is_some(),
            _ => false,
        };
        if again {
            self.gone = true;
            events.push(LiveEvent::Lost("bad data from the server".into()));
            return;
        }
        let view = &mut self.view;
        match message {
            ToPc::Challenge { nonce, motd } => {
                view.motd = motd;
                if self.launcher {
                    return self.answer_challenge(nonce, events);
                }
                let proof = live::proof(&nonce, self.key.verifying_key().as_bytes());
                ToServer::Prove(self.key.sign(&proof).to_bytes()).send(&mut self.conn);
            }
            // A launcher's: the key to seal its passwords to, which comes
            // before the challenge.
            ToPc::SealKey { key, server, sig } => {
                if self.launcher {
                    self.seal_key = Some((key, server, sig));
                }
            }
            ToPc::Accounts(accounts) => {
                events.push(LiveEvent::Accounts {
                    answers: accounts.answers,
                    ok: accounts.ok,
                    text: accounts.text.clone(),
                });
                view.accounts = Some(accounts);
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
                if view.launcher_game.as_ref().is_some_and(|g| g.id == over.id) {
                    view.launcher_game = None;
                }
                self.keep_card(&over.card);
                events.push(LiveEvent::MatchOver(over));
            }
            ToPc::CustomOpen { party, leader, map } => {
                events.push(LiveEvent::CustomOpen { party, leader, map });
            }
            ToPc::LauncherMatch(game) => {
                view.launcher_game = Some(game.clone());
                events.push(LiveEvent::LauncherMatch(game));
            }
            ToPc::Friends(friends) => {
                view.friends = friends;
                events.push(LiveEvent::Friends);
            }
            ToPc::ServiceRecord(record) => events.push(LiveEvent::ServiceRecord(record)),
            // Only relay legs are told they're linked.
            ToPc::Linked => {}
        }
    }

    /// A launcher answers the challenge `nonce`, once the server proved who
    /// it is: with the password, if it has one, sealed; else as the game
    /// does.
    fn answer_challenge(&mut self, nonce: [u8; 32], events: &mut Vec<LiveEvent>) {
        let proven = self.seal_key.take().filter(|(key, server, sig)| {
            let Ok(server) = VerifyingKey::from_bytes(server) else {
                return false;
            };
            let proof = seal::key_proof(&nonce, key);
            server
                .verify_strict(&proof, &Signature::from_bytes(sig))
                .is_ok()
        });
        let Some((key, server, _)) = proven else {
            self.gone = true;
            events.push(LiveEvent::Lost(SERVER_UNPROVEN.into()));
            return;
        };
        if let Some(was) = self.access.pin.filter(|&pin| pin != server) {
            self.gone = true;
            events.push(LiveEvent::KeyChanged { was, now: server });
            return;
        }
        self.view.server_key = Some(server);
        // Any other key this connection is offered must be this one.
        self.access.pin = Some(server);
        self.seal = Some((key, nonce));
        let me = self.key.verifying_key().to_bytes();
        // The password goes once, and isn't kept after it's sealed.
        let Some(typed) = self.access.password.take() else {
            let signature = self.key.sign(&live::proof(&nonce, &me)).to_bytes();
            return ToServer::Prove(signature).send(&mut self.conn);
        };
        let aad = seal::aad(seal::SIGN_IN, &nonce, &me, self.gamertag.as_bytes());
        let Some(password) = seal_passwords(&key, &aad, &[&typed]) else {
            // Too long to be anyone's.
            self.gone = true;
            events.push(LiveEvent::Refused(live::WRONG_PASSWORD.into()));
            return;
        };
        let computer = cut(&self.access.computer, live::MAX_NAME);
        let proof = live::password_proof(&nonce, &me, &self.gamertag, &computer, &password);
        let signature = self.key.sign(&proof).to_bytes();
        let prove = ToServer::ProvePassword {
            signature,
            computer,
            password,
        };
        prove.send(&mut self.conn);
    }

    /// Ask the server to change the accounts (or list them again), sealing
    /// any passwords to this connection's key. The answer comes as
    /// `LiveEvent::Accounts` whose `answers` is the number returned. None if
    /// we're not signed in on a launcher, or a password is too long to be
    /// anyone's.
    pub fn ask_account(&mut self, ask: AccountAsk) -> Option<u32> {
        if !self.signed_in() {
            return None;
        }
        let (key, nonce) = self.seal?;
        let me = self.key.verifying_key().to_bytes();
        let sealed = |purpose: u8, target: &[u8], passwords: &[&Secret]| {
            let aad = seal::aad(purpose, &nonce, &me, target);
            seal_passwords(&key, &aad, passwords)
        };
        let op = match ask {
            AccountAsk::List => AccountOp::List,
            AccountAsk::Create { gamertag, password } => {
                let gamertag = cut(&gamertag, live::MAX_NAME);
                let password = sealed(seal::CREATE, gamertag.as_bytes(), &[&password])?;
                AccountOp::Create { gamertag, password }
            }
            AccountAsk::SetPassword { account, password } => {
                let target = account.to_le_bytes();
                let password = sealed(seal::SET_PASSWORD, &target, &[&password])?;
                AccountOp::SetPassword { account, password }
            }
            AccountAsk::SignOut { account } => AccountOp::SignOut { account },
            AccountAsk::Remove { account } => AccountOp::Remove { account },
            AccountAsk::ChangePassword { old, new } => {
                let target = self.account()?.to_le_bytes();
                let passwords = sealed(seal::CHANGE_PASSWORD, &target, &[&old, &new])?;
                AccountOp::ChangePassword { passwords }
            }
        };
        self.account_seq += 1;
        let seq = self.account_seq;
        let sig = self.key.sign(&live::account_proof(&nonce, seq, &op));
        let sig = sig.to_bytes();
        self.send(ToServer::Account { seq, op, sig });
        Some(seq)
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

/// `passwords`, as typed, sealed to `key` for `aad`. None if one is too
/// long to be anyone's.
fn seal_passwords(key: &[u8; 32], aad: &[u8], passwords: &[&Secret]) -> Option<Sealed> {
    let typed: Vec<Secret> = passwords
        .iter()
        .map(|p| Secret::new(password::normalize(p.as_str())))
        .collect();
    let typed: Vec<&str> = typed.iter().map(Secret::as_str).collect();
    let plain = seal::pad(&typed)?;
    seal::seal(key, aad, &plain)
}

/// `text` cut to at most `max` bytes, on a character's edge.
fn cut(text: &str, max: usize) -> String {
    let mut text = text.to_string();
    while text.len() > max {
        text.pop();
    }
    text
}
