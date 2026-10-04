//! The online service. PCs sign in to it over a control link, and it keeps
//! their accounts. It runs on one thread: the caller hands it each new
//! connection (`accept`) and calls `poll` often, passing the time in
//! seconds since it started, so tests can run minutes in an instant.
//!
//! Signing in: a PC sends LOGIN with its public key, the server answers
//! with a random challenge, and the PC signs it (PROVE). An account is
//! named by its key, so only the PC holding the key can sign in as it, and
//! no two accounts have the same gamertag, whatever the case. Then the PC
//! and the server ping each other every second, and each gives up on the
//! other after 15 seconds without a word.
//!
//! Each time a player signs in they get their account on a stat card the
//! server signed (see `card`), and show it the next time. A card newer than
//! what the server has brings the account back, after the server lost its
//! data folder, say.

use crate::card;
use crate::playlists::{self, Playlist};
use crate::store::{self, Account};
use ed25519_dalek::{Signature, SigningKey, VerifyingKey};
use h2net::live::{self, Login, PlaylistInfo, ToPc, ToServer, Welcome};
use h2net::Connection;
use h2sim::game::{clean_name, Look};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::net::IpAddr;
use std::path::{Path, PathBuf};

/// Most connections at once.
pub const MAX_CONNECTIONS: usize = 500;
/// Most sign-ins from one address in a minute.
pub const SIGN_INS_PER_MINUTE: usize = 10;
/// A relay leg waits this long (seconds) for its other end.
const LEG_WAIT: f64 = 20.0;
/// A dropped connection is kept this long (seconds), so what was last sent
/// to it (why it was turned away, say) gets there.
const LINGER: f64 = 2.0;
/// Round trips kept, to take the middle of.
const ROUND_TRIPS: usize = 9;
/// Pings that can be on their way back at once.
const PINGS: usize = 16;

/// Where a connection came in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// `/live`: a PC's control link.
    Live,
    /// `/link`: one leg of a relayed game.
    Link,
}

/// A PC's control link.
struct Pc {
    conn: Connection,
    ip: IpAddr,
    /// When it connected, and when it was last heard from.
    since: f64,
    heard: f64,
    /// While signing in: what it signed in with, and the challenge it must
    /// sign.
    login: Option<(Login, [u8; 32])>,
    /// Signed in as this account.
    account: Option<u64>,
    /// The maps the PC has (file name and hash).
    maps: Vec<(String, u64)>,
    /// Splitscreen guests playing on it.
    guests: u8,
    /// The next ping's number and when the last went; pings on their way
    /// back (number, when sent); and the latest round trips (milliseconds).
    ping: u32,
    pinged: f64,
    pings: VecDeque<(u32, f64)>,
    round_trips: VecDeque<u32>,
    /// Why it's being dropped, once it is.
    gone: Option<String>,
}

impl Pc {
    fn new(conn: Connection, ip: IpAddr, now: f64) -> Pc {
        Pc {
            conn,
            ip,
            since: now,
            heard: now,
            login: None,
            account: None,
            maps: Vec::new(),
            guests: 0,
            ping: 0,
            pinged: now,
            pings: VecDeque::new(),
            round_trips: VecDeque::new(),
            gone: None,
        }
    }

    /// A ping came back.
    fn pong(&mut self, n: u32, now: f64) {
        if let Some(i) = self.pings.iter().position(|&(p, _)| p == n) {
            let (_, sent) = self.pings.remove(i).unwrap_or((n, now));
            if self.round_trips.len() == ROUND_TRIPS {
                self.round_trips.pop_front();
            }
            self.round_trips
                .push_back(((now - sent) * 1000.0).round() as u32);
        }
    }
}

/// A relay leg, waiting for its other end.
struct Leg {
    conn: Connection,
    since: f64,
    /// Said which leg it is (LINK_HELLO).
    hello: bool,
}

pub struct Server {
    /// The data folder.
    dir: PathBuf,
    /// Signs stat cards.
    key: SigningKey,
    playlists: Vec<Playlist>,
    accounts: BTreeMap<u64, Account>,
    /// Sent with every challenge.
    motd: String,
    pcs: Vec<Pc>,
    legs: Vec<Leg>,
    /// Dropped connections, and when to let them go.
    lingering: Vec<(Connection, f64)>,
    /// When each address started signing in, in the last minute.
    sign_ins: HashMap<IpAddr, Vec<f64>>,
    /// The Unix time when the server's clock read 0.
    epoch: u64,
}

impl Server {
    /// A server keeping its accounts in the folder `dir` (made if need be),
    /// and taking its playlists from `playlists.txt` and its message of the
    /// day from `motd.txt` there, if they're there. Its stat cards are
    /// signed with a key made from `secret` (H2LIVE_SECRET), or else kept in
    /// `secret.txt` there.
    pub fn open(dir: &Path, secret: Option<&str>) -> Result<Server, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let key = card::server_key(dir, secret)?;
        let playlists = playlists::load(&dir.join("playlists.txt"))?;
        let accounts = store::load(&dir.join("accounts.txt"))?;
        let motd = std::fs::read_to_string(dir.join("motd.txt")).unwrap_or_default();
        let epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Ok(Server {
            dir: dir.to_path_buf(),
            key,
            playlists,
            accounts: accounts.into_iter().map(|a| (a.id, a)).collect(),
            motd: motd.trim().to_string(),
            pcs: Vec::new(),
            legs: Vec::new(),
            lingering: Vec::new(),
            sign_ins: HashMap::new(),
            epoch,
        })
    }

    /// Connections open now (control links and relay legs).
    pub fn connections(&self) -> usize {
        self.pcs.len() + self.legs.len()
    }

    /// Players signed in now.
    pub fn players_online(&self) -> usize {
        self.pcs.iter().filter(|pc| pc.account.is_some()).count()
    }

    pub fn account(&self, id: u64) -> Option<&Account> {
        self.accounts.get(&id)
    }

    /// The middle of a signed-in player's recent round trips to the server,
    /// in milliseconds.
    pub fn round_trip(&self, account: u64) -> Option<u32> {
        let pc = &self.pcs[self.pc_of(account)?];
        let mut times: Vec<u32> = pc.round_trips.iter().copied().collect();
        times.sort_unstable();
        times.get(times.len() / 2).copied()
    }

    /// Take a new connection that came in on `route` from `ip`, at time
    /// `now`.
    pub fn accept(&mut self, mut conn: Connection, route: Route, ip: IpAddr, now: f64) {
        if self.connections() >= MAX_CONNECTIONS {
            if route == Route::Live {
                ToPc::Refused(live::SERVER_FULL.into()).send(&mut conn);
            }
            return self.linger(conn, now);
        }
        match route {
            Route::Link => self.legs.push(Leg {
                conn,
                since: now,
                hello: false,
            }),
            Route::Live => {
                let recent = self.sign_ins.entry(ip).or_default();
                recent.retain(|&t| now - t < 60.0);
                if recent.len() >= SIGN_INS_PER_MINUTE {
                    ToPc::Refused(live::TOO_MANY_SIGN_INS.into()).send(&mut conn);
                    return self.linger(conn, now);
                }
                recent.push(now);
                self.pcs.push(Pc::new(conn, ip, now));
            }
        }
    }

    /// Read and answer everything that arrived, ping, and drop connections
    /// that went quiet, at time `now`.
    pub fn poll(&mut self, now: f64) {
        for k in 0..self.pcs.len() {
            self.read_pc(k, now);
        }
        self.read_legs(now);
        self.keep_up(now);
        let mut k = 0;
        while k < self.pcs.len() {
            if self.pcs[k].gone.is_some() {
                let pc = self.pcs.remove(k);
                self.linger(pc.conn, now);
            } else {
                k += 1;
            }
        }
        for pc in &mut self.pcs {
            // A failure shows up when reading, next time.
            let _ = pc.conn.flush();
        }
        self.lingering
            .retain_mut(|(conn, until)| conn.flush().is_ok() && now < *until);
        self.sign_ins
            .retain(|_, times| times.last().is_some_and(|&t| now - t < 60.0));
    }

    /// Keep a dropped connection a moment, to send what's left.
    fn linger(&mut self, mut conn: Connection, now: f64) {
        let _ = conn.flush();
        self.lingering.push((conn, now + LINGER));
    }

    /// The PC signed in as `account`.
    fn pc_of(&self, account: u64) -> Option<usize> {
        self.pcs.iter().position(|pc| pc.account == Some(account))
    }

    /// Send a signed-in player `message`.
    fn tell(&mut self, account: u64, message: &ToPc) {
        if let Some(k) = self.pc_of(account) {
            message.send(&mut self.pcs[k].conn);
        }
    }

    /// Drop PC `k`, signing it out.
    fn drop_pc(&mut self, k: usize, why: &str) {
        let pc = &mut self.pcs[k];
        if pc.gone.is_some() {
            return;
        }
        pc.gone = Some(why.to_string());
        match pc.account.take().and_then(|id| self.accounts.get(&id)) {
            Some(account) => println!("live: {} signed out: {why}", account.gamertag),
            None => println!("live: {} dropped: {why}", pc.ip),
        }
    }

    /// Turn PC `k` away, telling it why.
    fn refuse(&mut self, k: usize, why: &str) {
        ToPc::Refused(why.into()).send(&mut self.pcs[k].conn);
        self.drop_pc(k, why);
    }

    fn read_pc(&mut self, k: usize, now: f64) {
        let messages = match self.pcs[k].conn.receive() {
            Ok(m) => m,
            Err(why) => return self.drop_pc(k, &why),
        };
        if !messages.is_empty() {
            self.pcs[k].heard = now;
        }
        for (kind, body) in messages {
            if self.pcs[k].gone.is_some() {
                break;
            }
            if body.len() >= live::MAX_MESSAGE {
                self.drop_pc(k, "sent too much at once");
            } else if kind == live::kind::LOGIN
                && live::login_protocol(&body) != Some(h2net::PROTOCOL)
            {
                self.refuse(k, live::UPDATE_YOUR_GAME);
            } else {
                let handled = match ToServer::read(kind, &body) {
                    Ok(message) => self.handle(k, message, now),
                    Err(_) => false,
                };
                if !handled {
                    self.drop_pc(k, &format!("unexpected message {kind}"));
                }
            }
        }
    }

    /// Act on a message from PC `k`. False if it shouldn't have sent it.
    fn handle(&mut self, k: usize, message: ToServer, now: f64) -> bool {
        let pc = &mut self.pcs[k];
        match (pc.account, message) {
            (_, ToServer::Ping(n)) => ToPc::Pong(n).send(&mut pc.conn),
            (_, ToServer::Pong(n)) => pc.pong(n, now),
            (None, ToServer::Login(login)) if pc.login.is_none() => {
                let mut nonce = [0; 32];
                getrandom::fill(&mut nonce).expect("random numbers");
                let motd = self.motd.clone();
                ToPc::Challenge { nonce, motd }.send(&mut pc.conn);
                pc.login = Some((login, nonce));
            }
            (None, ToServer::Prove(signature)) => {
                let Some((login, nonce)) = pc.login.take() else {
                    return false;
                };
                if signed(&login.key, &nonce, &signature) {
                    self.sign_in(k, login, now);
                } else {
                    self.refuse(k, live::BAD_SIGNATURE);
                }
            }
            (Some(me), ToServer::Profile { gamertag, look }) => {
                self.profile(me, &gamertag, look);
            }
            // Matchmaking and matches aren't served yet.
            (
                Some(_),
                ToServer::Search(_)
                | ToServer::Cancel
                | ToServer::Custom
                | ToServer::CustomMap(_)
                | ToServer::Hosting(_)
                | ToServer::Result { .. }
                | ToServer::LeftMatch { .. }
                | ToServer::Back,
            ) => {}
            _ => return false,
        }
        true
    }

    /// Sign PC `k` in, now that it proved it holds `login.key`.
    fn sign_in(&mut self, k: usize, login: Login, now: f64) {
        let id = store::account_id(&login.key);
        let existing = self.accounts.get(&id);
        // A card newer than what's here brings back what this server lost.
        let restored = card::verify(&login.card, &self.key.verifying_key())
            .filter(|a| a.key == login.key && a.seq > existing.map_or(0, |e| e.seq));
        if let Some(a) = &restored {
            println!("live: {} restored from a stat card", a.gamertag);
        }
        let mut account = restored
            .or_else(|| existing.cloned())
            .unwrap_or_else(|| Account::new(login.key, self.epoch + now as u64));
        // The gamertag asked for if it's free; otherwise the one it had.
        let names = [
            clean_name(&login.gamertag),
            account.gamertag.clone(),
            existing.map_or(String::new(), |a| a.gamertag.clone()),
        ];
        let Some(gamertag) = names.into_iter().find(|n| self.free(n, id)) else {
            return self.refuse(k, live::GAMERTAG_TAKEN);
        };
        account.gamertag = gamertag;
        account.look = login.look;
        self.update(account);
        // One PC per account: the newest.
        if let Some(j) = self.pc_of(id) {
            self.refuse(j, live::SIGNED_IN_ELSEWHERE);
        }
        let pc = &mut self.pcs[k];
        pc.account = Some(id);
        pc.maps = login.maps;
        pc.guests = login.guests;
        // The first ping goes at once.
        pc.pinged = now - live::PING_EVERY;
        println!(
            "live: {} signed in from {}",
            self.accounts[&id].gamertag, pc.ip
        );
        self.welcome(id);
        let playlists = ToPc::Playlists(self.playlists_for(id));
        self.tell(id, &playlists);
    }

    /// A player asks for a new gamertag or look.
    fn profile(&mut self, me: u64, gamertag: &str, look: Look) {
        let Some(mut account) = self.accounts.get(&me).cloned() else {
            return;
        };
        let wanted = clean_name(gamertag);
        if self.free(&wanted, me) {
            account.gamertag = wanted;
        } else {
            self.tell(me, &ToPc::Notice(live::GAMERTAG_TAKEN.into()));
        }
        account.look = look;
        if self.update(account) {
            self.welcome(me);
        }
    }

    /// No account but `id`'s has gamertag `name`, whatever the case.
    fn free(&self, name: &str, id: u64) -> bool {
        let taken = |a: &Account| a.id != id && a.gamertag.eq_ignore_ascii_case(name);
        !name.is_empty() && !self.accounts.values().any(taken)
    }

    /// Keep `account` as it is now, if it changed: with a new seq, saved
    /// to accounts.txt. True if it changed.
    fn update(&mut self, mut account: Account) -> bool {
        if self.accounts.get(&account.id) == Some(&account) {
            return false;
        }
        account.seq += 1;
        self.accounts.insert(account.id, account);
        let path = self.dir.join("accounts.txt");
        if let Err(e) = store::save(&path, self.accounts.values()) {
            println!("live: can't save {}: {e}", path.display());
        }
        true
    }

    /// Tell a player who they're signed in as.
    fn welcome(&mut self, id: u64) {
        let Some(account) = self.accounts.get(&id) else {
            return;
        };
        let levels = self.playlists.iter().filter_map(|p| {
            let s = account.stats(&p.key)?;
            Some((p.id, s.rank.level, s.games))
        });
        let welcome = Welcome {
            account: id,
            gamertag: account.gamertag.clone(),
            card: card::sign(account, &self.key),
            best: account.best_level(),
            levels: levels.collect(),
        };
        self.tell(id, &ToPc::Welcome(welcome));
    }

    /// The playlists, as a player sees them.
    fn playlists_for(&self, id: u64) -> Vec<PlaylistInfo> {
        let account = self.accounts.get(&id);
        self.playlists
            .iter()
            .map(|p| {
                let stats = account.and_then(|a| a.stats(&p.key));
                PlaylistInfo {
                    id: p.id,
                    key: p.key.clone(),
                    name: p.name.clone(),
                    ranked: p.ranked,
                    teams: p.teams,
                    guests: p.guests,
                    min: p.min,
                    max: p.max,
                    party_max: p.party_max,
                    searching: 0,
                    playing: 0,
                    level: match stats {
                        _ if !p.ranked => 0,
                        Some(s) => s.rank.level,
                        None => 1,
                    },
                }
            })
            .collect()
    }

    /// Ping signed-in PCs, and drop those that went quiet or never signed
    /// in.
    fn keep_up(&mut self, now: f64) {
        for k in 0..self.pcs.len() {
            let pc = &mut self.pcs[k];
            if pc.gone.is_some() {
                continue;
            }
            if now - pc.heard > live::TIMEOUT {
                self.drop_pc(k, "timed out");
            } else if pc.account.is_none() && now - pc.since > live::TIMEOUT {
                self.drop_pc(k, "never signed in");
            } else if pc.account.is_some() && now - pc.pinged >= live::PING_EVERY {
                ToPc::Ping(pc.ping).send(&mut pc.conn);
                if pc.pings.len() == PINGS {
                    pc.pings.pop_front();
                }
                pc.pings.push_back((pc.ping, now));
                pc.ping = pc.ping.wrapping_add(1);
                pc.pinged = now;
            }
        }
    }

    /// Relay legs say which they are, then wait for their other end; one
    /// that says anything else, or waits too long, is dropped. (They're
    /// paired up once the server has matches to relay.)
    fn read_legs(&mut self, now: f64) {
        self.legs.retain_mut(|leg| {
            let Ok(messages) = leg.conn.receive() else {
                return false;
            };
            for (kind, body) in messages {
                let hello = matches!(ToServer::read(kind, &body), Ok(ToServer::LinkHello { .. }));
                if leg.hello || !hello {
                    return false;
                }
                leg.hello = true;
            }
            now - leg.since < LEG_WAIT
        });
    }
}

/// `signature` is `key`'s, over the proof for `nonce`.
fn signed(key: &[u8; 32], nonce: &[u8; 32], signature: &[u8; 64]) -> bool {
    let Ok(key) = VerifyingKey::from_bytes(key) else {
        return false;
    };
    let signature = Signature::from_bytes(signature);
    key.verify_strict(&live::proof(nonce, key.as_bytes()), &signature)
        .is_ok()
}

#[cfg(test)]
mod tests;
