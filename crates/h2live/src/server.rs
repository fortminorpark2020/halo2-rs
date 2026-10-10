//! The online service. PCs sign in to it over a control link, and it keeps
//! their accounts and their parties (see `party`), finds them matches and
//! sees them through (`matches`), and passes their games between them
//! (`relay`). It runs on one thread: the caller hands it each new
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
//! Two programs sign in (see `h2net::live`): the game, taken only at the
//! server's own game `PROTOCOL`, and the launcher, taken at the service's
//! `LIVE_PROTOCOL` whatever the game's `PROTOCOL` is. Each sees only its
//! own playlists and players, and parties never mix them. The launcher's
//! matches are played on MCC's engine through the UDP relay, which this
//! server opens a room on for each of them (see `matches`).
//!
//! Each time a player signs in they get their account on a stat card the
//! server signed (see `card`), and show it the next time. A card newer than
//! what the server has brings the account back, after the server lost its
//! data folder, say.

use crate::card;
use crate::matchmaker::Matchmaker;
use crate::playlists::{self, Playlist};
use crate::store::{self, Account};
use ed25519_dalek::{Signature, SigningKey, VerifyingKey};
use h2net::live::{
    self, Activity, ClientKind, Login, LoginClient, LoginHeader, PlaylistInfo, Privacy, ToPc,
    ToServer, Welcome,
};
use h2net::Connection;
use h2relay::RelayHandle;
use h2sim::game::{clean_name, Look};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::Write;
use std::net::{IpAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

mod matches;
mod party;
mod relay;

/// Most connections at once.
pub const MAX_CONNECTIONS: usize = 500;
/// Most sign-ins from one address in a minute.
pub const SIGN_INS_PER_MINUTE: usize = 10;
/// A dropped connection is kept this long (seconds), so what was last sent
/// to it (why it was turned away, say) gets there.
const LINGER: f64 = 2.0;
/// Round trips kept, to take the middle of.
const ROUND_TRIPS: usize = 9;
/// Pings that can be on their way back at once.
const PINGS: usize = 16;
/// A PC's gamertag and look change at most this often (seconds), since
/// each change rewrites accounts.txt.
const PROFILE_EVERY: f64 = 1.0;
/// Log lines waiting to be written out, at most (once they go out in the
/// background): while nothing takes them, more are dropped.
const LOG_BACKLOG: usize = 1000;
/// How long to wait for them to go out, at most, before the program ends.
const LOG_FLUSH: Duration = Duration::from_secs(1);

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
    /// Signed in as this account, in this party, from this program.
    account: Option<u64>,
    party: u64,
    client: ClientKind,
    /// The maps the PC has (file name and hash).
    maps: Vec<(String, u64)>,
    /// Splitscreen guests playing on it.
    guests: u8,
    /// The gamertag and look it last asked for, not yet taken on, and when
    /// it last changed them.
    profile: Option<(String, Look)>,
    profiled: f64,
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
            party: 0,
            client: ClientKind::Viewer,
            maps: Vec::new(),
            guests: 0,
            profile: None,
            profiled: f64::NEG_INFINITY,
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

/// A party of players, which goes into matches whole.
struct Party {
    leader: u64,
    privacy: Privacy,
    /// Members, the longest-standing first.
    members: Vec<u64>,
    /// Invites not yet answered: who to, and who from.
    invited: Vec<(u64, u64)>,
    /// Members the leader removed, who need an invite to come back.
    booted: Vec<u64>,
    /// What it's doing, and the playlist it searches or plays (or last
    /// played, back in its lobby).
    activity: Activity,
    playlist: u8,
    /// The map of its last match, which its next won't be on.
    previous_map: Option<String>,
    /// The map of its custom game, while it plays one.
    custom_map: String,
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
    /// Relay legs waiting for their other end, legs joined, and links
    /// given out whose legs haven't both come (see `relay`); hosts'
    /// fan-out legs, and those given out that haven't come (the match or
    /// custom game, and the host).
    legs: Vec<relay::Leg>,
    links: Vec<relay::Link>,
    tokens: HashMap<[u8; 16], relay::Token>,
    fanouts: Vec<relay::Fanout>,
    fanout_tokens: HashMap<[u8; 16], (u64, u64)>,
    /// Bytes relayed for each match (or party's custom game) so far.
    relayed: HashMap<u64, u64>,
    matchmaker: Matchmaker,
    /// Matches formed and not yet over (see `matches`).
    matches: Vec<matches::Match>,
    /// People searching and playing each playlist, as players were last
    /// told, and when.
    counts_sent: Vec<(u16, u16)>,
    playlists_sent: f64,
    /// Dropped connections, and when to let them go.
    lingering: Vec<(Connection, f64)>,
    /// Ended launcher matches' relay rooms, and when to close them.
    closing_rooms: Vec<(u64, f64)>,
    /// When each address started signing in, in the last minute.
    sign_ins: HashMap<IpAddr, Vec<f64>>,
    parties: HashMap<u64, Party>,
    /// The last party number given out.
    party_ids: u64,
    /// Parties whose members haven't been told how they changed.
    changed_parties: Vec<u64>,
    /// Whether the ONLINE list changed since it was last sent, and when
    /// that was.
    online_changed: bool,
    online_sent: f64,
    /// The Unix time when the server's clock read 0.
    epoch: u64,
    /// The UDP relay the launcher's matches are played through, if it's on.
    relay: Option<RelayHandle>,
    /// The game's `PROTOCOL` it takes: this build's. (A test pretends it
    /// was bumped.)
    game_protocol: u32,
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
        let mut seed = [0; 8];
        getrandom::fill(&mut seed).expect("random numbers");
        Ok(Server {
            dir: dir.to_path_buf(),
            key,
            matchmaker: Matchmaker::new(playlists.clone(), u64::from_le_bytes(seed)),
            counts_sent: vec![(0, 0); playlists.len()],
            playlists,
            accounts: accounts.into_iter().map(|a| (a.id, a)).collect(),
            motd: motd.trim().to_string(),
            pcs: Vec::new(),
            legs: Vec::new(),
            links: Vec::new(),
            tokens: HashMap::new(),
            fanouts: Vec::new(),
            fanout_tokens: HashMap::new(),
            relayed: HashMap::new(),
            matches: Vec::new(),
            playlists_sent: f64::NEG_INFINITY,
            lingering: Vec::new(),
            closing_rooms: Vec::new(),
            sign_ins: HashMap::new(),
            parties: HashMap::new(),
            party_ids: 0,
            changed_parties: Vec::new(),
            online_changed: false,
            online_sent: f64::NEG_INFINITY,
            epoch,
            relay: None,
            game_protocol: h2net::PROTOCOL,
        })
    }

    /// Play the launcher's matches through `relay` (h2relay's, running
    /// beside this server with `Admission::Issued`): a room on it for each
    /// match, with a key for each player. Without one, launchers can sign in
    /// and form parties but not search.
    pub fn set_relay(&mut self, relay: RelayHandle) {
        self.relay = Some(relay);
    }

    /// Connections open now (control links and relay legs).
    pub fn connections(&self) -> usize {
        self.pcs.len() + self.legs.len() + 2 * self.links.len() + self.fanouts.len()
    }

    /// The TCP connections of those open now, to wait for something to
    /// come on.
    pub fn streams(&self) -> impl Iterator<Item = &TcpStream> {
        let pcs = self.pcs.iter().map(|pc| &pc.conn);
        pcs.chain(self.relay_conns()).filter_map(Connection::stream)
    }

    /// Players signed in now.
    pub fn players_online(&self) -> usize {
        self.pcs.iter().filter(|pc| pc.account.is_some()).count()
    }

    /// Matches formed and not yet over.
    pub fn matches_in_progress(&self) -> usize {
        self.matches.len()
    }

    /// Parties playing custom games now.
    pub fn custom_games(&self) -> usize {
        let custom = |p: &&Party| p.activity == Activity::Custom;
        self.parties.values().filter(custom).count()
    }

    /// The account with this id, as the server keeps it.
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
            Route::Link => self.add_leg(conn, ip, now),
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

    /// Read and answer everything that arrived, pass games on, find
    /// matches and see them through, ping, drop connections that went
    /// quiet, and tell players what changed, at time `now`.
    pub fn poll(&mut self, now: f64) {
        for k in 0..self.pcs.len() {
            self.read_pc(k, now);
        }
        self.change_profiles(now);
        self.read_legs(now);
        self.fan_out(now);
        self.relay(now);
        self.keep_up(now);
        self.matchmake(now);
        self.run_matches(now);
        let mut k = 0;
        while k < self.pcs.len() {
            if self.pcs[k].gone.is_some() {
                let pc = self.pcs.remove(k);
                self.linger(pc.conn, now);
            } else {
                k += 1;
            }
        }
        self.send_parties();
        self.send_online(now);
        self.send_playlists(now);
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

    /// The program a signed-in player plays on.
    fn client_of(&self, account: u64) -> ClientKind {
        self.pc_of(account)
            .map_or(ClientKind::Viewer, |k| self.pcs[k].client)
    }

    /// Send a signed-in player `message`.
    fn tell(&mut self, account: u64, message: &ToPc) {
        if let Some(k) = self.pc_of(account) {
            message.send(&mut self.pcs[k].conn);
        }
    }

    /// Drop PC `k`, signing it out of its party and out of any match.
    fn drop_pc(&mut self, k: usize, why: &str) {
        let pc = &mut self.pcs[k];
        if pc.gone.is_some() {
            return;
        }
        pc.gone = Some(why.to_string());
        let (account, party, ip) = (pc.account.take(), pc.party, pc.ip);
        match account.and_then(|id| self.accounts.get(&id)) {
            Some(a) => log(format_args!("live: {} signed out: {why}", a.gamertag)),
            None => log(format_args!("live: {ip} dropped: {why}")),
        }
        if let Some(id) = account {
            self.remove_member(party, id);
            self.online_changed = true;
            self.gone(id);
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
            let refusal = match kind {
                live::kind::LOGIN => self.login_refusal(&body),
                _ => None,
            };
            if body.len() >= live::MAX_MESSAGE {
                self.drop_pc(k, "sent too much at once");
            } else if let Some(why) = refusal {
                self.refuse(k, why);
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

    /// Why a PC that sent LOGIN `body` is turned away for its version, if
    /// it is: the game at another `PROTOCOL` than this server's, or a
    /// versioned LOGIN at another `LIVE_PROTOCOL`. A launcher's own version
    /// is no matter.
    fn login_refusal(&self, body: &[u8]) -> Option<&'static str> {
        match live::login_header(body) {
            Some(LoginHeader::Game { protocol }) if protocol == self.game_protocol => None,
            Some(LoginHeader::Versioned { live, .. }) if live == live::LIVE_PROTOCOL => {
                match ToServer::read(live::kind::LOGIN, body) {
                    Ok(ToServer::Login(Login {
                        client: LoginClient::Viewer { protocol },
                        ..
                    })) if protocol != self.game_protocol => Some(live::UPDATE_YOUR_GAME),
                    // (One that doesn't read is dropped as it's read.)
                    _ => None,
                }
            }
            Some(LoginHeader::Versioned {
                client: live::CLIENT_LAUNCHER,
                ..
            }) => Some(live::UPDATE_YOUR_LAUNCHER),
            _ => Some(live::UPDATE_YOUR_GAME),
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
            (Some(_), ToServer::Profile { gamertag, look }) => pc.profile = Some((gamertag, look)),
            (Some(me), ToServer::Invite(who)) => self.invite(me, who),
            (Some(me), ToServer::Accept(id) | ToServer::JoinParty(id)) => self.join_party(me, id),
            (Some(me), ToServer::Decline(id)) => self.decline(me, id),
            (Some(me), ToServer::LeaveParty) => self.leave_party(me),
            (Some(me), ToServer::Kick(who)) => self.kick(me, who),
            (Some(me), ToServer::Promote(who)) => self.promote(me, who),
            (Some(me), ToServer::Privacy(privacy)) => self.set_privacy(me, privacy),
            (Some(me), ToServer::Guests(n)) => self.set_guests(me, n),
            (Some(me), ToServer::Search(playlist)) => self.search(me, playlist, now),
            (Some(me), ToServer::Cancel) => self.cancel(me),
            (Some(me), ToServer::Custom) => self.custom(me),
            (Some(me), ToServer::CustomMap(map)) => self.custom_map(me, &map),
            (Some(me), ToServer::Hosting(id)) => self.hosting(me, id, now),
            (Some(me), ToServer::Result { id, players }) => self.result(me, id, players),
            (Some(me), ToServer::Joined(id)) => self.joined(me, id),
            (Some(me), ToServer::LauncherResult(result)) => self.launcher_result(me, result),
            (Some(me), ToServer::LeftMatch { id, host_lost }) => {
                self.left_match(me, id, host_lost);
            }
            (Some(me), ToServer::Back) => self.back(me),
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
            log(format_args!(
                "live: {} restored from a stat card",
                a.gamertag
            ));
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
        pc.client = login.client.kind();
        pc.maps = login.maps;
        pc.guests = login.guests;
        // The first ping goes at once.
        pc.pinged = now - live::PING_EVERY;
        let program = match &login.client {
            LoginClient::Viewer { .. } => String::new(),
            LoginClient::Launcher { version } => format!(" (launcher {})", printable(version)),
        };
        log(format_args!(
            "live: {} signed in from {}{program}",
            self.accounts[&id].gamertag, pc.ip
        ));
        self.welcome(id);
        let playlists = ToPc::Playlists(self.playlists_for(id, &self.playlist_counts()));
        self.tell(id, &playlists);
        self.new_party(id);
    }

    /// Take on the gamertags and looks PCs asked for, each PC's at most
    /// once a second.
    fn change_profiles(&mut self, now: f64) {
        for k in 0..self.pcs.len() {
            let pc = &mut self.pcs[k];
            if now - pc.profiled < PROFILE_EVERY {
                continue;
            }
            if let (Some(me), Some((gamertag, look))) = (pc.account, pc.profile.take()) {
                pc.profiled = now;
                self.profile(me, &gamertag, look);
            }
        }
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
            // Their party and everyone online see the change.
            if let Some(k) = self.pc_of(me) {
                self.party_changed(self.pcs[k].party);
            }
        }
    }

    /// No account but `id`'s has gamertag `name`, whatever the case.
    fn free(&self, name: &str, id: u64) -> bool {
        let taken = |a: &Account| a.id != id && a.gamertag.eq_ignore_ascii_case(name);
        !name.is_empty() && !self.accounts.values().any(taken)
    }

    /// Keep `account` as it is now, if it changed: with a new seq, saved
    /// to accounts.txt. True if it changed.
    fn update(&mut self, account: Account) -> bool {
        self.update_all(vec![account])
    }

    /// Keep each of `accounts` as it is now, as `update` does, saving
    /// accounts.txt once. True if any changed.
    fn update_all(&mut self, accounts: Vec<Account>) -> bool {
        let mut changed = false;
        for mut account in accounts {
            if self.accounts.get(&account.id) != Some(&account) {
                account.seq += 1;
                self.accounts.insert(account.id, account);
                changed = true;
            }
        }
        let path = self.dir.join("accounts.txt");
        if changed {
            if let Err(e) = store::save(&path, self.accounts.values()) {
                log(format_args!("live: can't save {}: {e}", path.display()));
            }
        }
        changed
    }

    /// Tell a player who they're signed in as, with their levels in their
    /// program's playlists.
    fn welcome(&mut self, id: u64) {
        let Some(account) = self.accounts.get(&id) else {
            return;
        };
        let client = self.client_of(id);
        let theirs = self.playlists.iter().filter(|p| p.client == client);
        let levels = theirs.filter_map(|p| {
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

    /// The playlists of a player's program as they see them, with how many
    /// people search and play each, and their maps.
    fn playlists_for(&self, id: u64, counts: &[(u16, u16)]) -> Vec<PlaylistInfo> {
        let account = self.accounts.get(&id);
        let client = self.client_of(id);
        self.playlists
            .iter()
            .zip(counts)
            .filter(|(p, _)| p.client == client)
            .map(|(p, &(searching, playing))| {
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
                    searching,
                    playing,
                    level: match stats {
                        _ if !p.ranked => 0,
                        Some(s) => s.rank.level,
                        None => 1,
                    },
                    maps: p.maps.clone(),
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
}

/// `text` as a log line can show it: anything but printable ASCII as `?`
/// (so what a PC sends can't start a line of its own).
fn printable(text: &str) -> String {
    let shown = |c: char| {
        if c.is_ascii_graphic() || c == ' ' {
            c
        } else {
            '?'
        }
    };
    text.chars().map(shown).collect()
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

/// Where log lines go once they go out in the background, and how many
/// have gone there, and been written out (or failed to be).
static LOG_LINES: OnceLock<SyncSender<String>> = OnceLock::new();
static LOGGED: AtomicUsize = AtomicUsize::new(0);
static WRITTEN: AtomicUsize = AtomicUsize::new(0);

/// Tell whoever runs the server something, on a line that starts with the
/// date and time (UTC).
pub fn log(line: impl std::fmt::Display) {
    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    say(format_args!("{} {line}", utc(unix)));
}

/// Tell whoever runs the server something, on a line of its own.
pub fn say(line: impl std::fmt::Display) {
    let line = format!("{line}\n");
    match LOG_LINES.get() {
        Some(lines) => {
            if lines.try_send(line).is_ok() {
                LOGGED.fetch_add(1, Ordering::SeqCst);
            }
        }
        None => print!("{line}"),
    }
}

/// From now on, write what's said out on a thread of its own, so the
/// server goes on when nothing takes it for a while (a console with text
/// selected, on Windows, or a pipe no one reads), or ever again.
pub fn log_in_background() {
    LOG_LINES.get_or_init(|| {
        let (lines, to_write) = mpsc::sync_channel::<String>(LOG_BACKLOG);
        std::thread::spawn(move || {
            let mut out = std::io::stdout();
            for line in to_write {
                // Whoever was reading may have gone.
                let _ = out.write_all(line.as_bytes());
                WRITTEN.fetch_add(1, Ordering::SeqCst);
            }
        });
        lines
    });
}

/// Wait (a moment at most) for what's been said to be written out, before
/// the program ends.
pub fn flush_log() {
    let start = Instant::now();
    while WRITTEN.load(Ordering::SeqCst) < LOGGED.load(Ordering::SeqCst)
        && start.elapsed() < LOG_FLUSH
    {
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A Unix time as a date and time, as "2026-10-04 12:34:56".
fn utc(unix: u64) -> String {
    let (days, seconds) = (unix / 86_400, unix % 86_400);
    // Howard Hinnant's `civil_from_days`, for years of March to February
    // in 400-year eras.
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let m = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * m + 2) / 5 + 1;
    let month = if m < 10 { m + 3 } else { m - 9 };
    let year = era * 400 + year_of_era + u64::from(month <= 2);
    let (h, min, s) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    format!("{year}-{month:02}-{day:02} {h:02}:{min:02}:{s:02}")
}

#[cfg(test)]
mod tests;
