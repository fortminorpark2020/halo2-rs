//! Friends lists and service records, for launcher players only: the game
//! (h2viewer) never sends or hears of them (a game PC sending one of these
//! messages is dropped, as for any it shouldn't send).
//!
//! A launcher player asks another by gamertag (FRIEND_REQUEST), looked up
//! as signing in does, among every account; the other accepts or declines
//! (FRIEND_ACCEPT, FRIEND_DECLINE), and either can end the friendship or
//! take back a request (FRIEND_REMOVE). The rules and `friends.txt` are in
//! `crate::friends`. The asker hears how it went (NOTICE); the one asked
//! hears of the request if they're on a launcher, at most once in 10
//! minutes for the same asker, so asking and taking it back can't nag them.
//! A decline or a removal isn't told: the other's list just changes. An
//! answer that finds nothing to change (the request was taken back a moment
//! before) gets no notice, and the list is sent again.
//!
//! Each launcher player has their whole list sent (FRIENDS) when it
//! changes, at most once a second: every second the server works out
//! everyone's presence (where they're signed in, gamertag, highest level,
//! what their party is doing and where, and how open and full it is), and
//! an account whose presence changed marks each of its friends, those it
//! has a request with either way, and itself, as needing a new list
//! (whether a friend's party can be joined depends on its own party too).
//! A request, an answer or a removal marks both players, signing in marks
//! the player, an invite marks the one invited (and any invite it pushed
//! out), and so does declining one, and a removal from a party marks any
//! removed player it pushed out. A list the same as the last one sent to
//! that PC isn't sent again.
//!
//! A service record (RECORD, answered with SERVICE_RECORD) is a player's
//! level, games, wins and tally in each ranked playlist of the asker's
//! program they have played.
//!
//! Friend actions are limited to 30 a minute per account, and RECORDs to
//! 20 in 10 seconds; more are ignored (the first friend action over, each
//! minute, with SLOW_DOWN). The times are kept per account, not per PC, so
//! signing in again doesn't start them afresh.

use super::{log, Pc, Server};
use crate::friends::{Asked, Refusal};
use h2net::live::{
    self, friend_notice, Activity, ClientKind, Friend, Online, PlaylistRecord, Privacy, Record,
    Relation, ServiceRecord, ToPc, ToServer, MAX_FRIEND_ENTRIES, MAX_PARTY, MAX_RECORD_PLAYLISTS,
};
use h2sim::game::clean_name;
use std::collections::{HashMap, VecDeque};

/// FRIENDS lists go at most this often (seconds), as ONLINE does.
const FRIENDS_EVERY: f64 = 1.0;
/// Friend actions an account may make in a minute.
pub const FRIEND_ACTIONS_PER_MINUTE: usize = 30;
/// RECORDs answered for an account in any 10 seconds.
pub const RECORDS_PER_10S: usize = 20;
/// ASKED_YOU goes to the same player for the same asker at most this often
/// (seconds). An estimate of what stops nagging; kept in memory only.
const ASKED_YOU_EVERY: f64 = 600.0;

/// A signed-in player as their friends see them: what, changed, means a
/// new list for each of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Presence {
    client: ClientKind,
    gamertag: String,
    best: u8,
    /// The rest only for launchers (zeros for the game's players, whose
    /// friends see only that they're on).
    activity: Activity,
    playlist: u8,
    map: String,
    variant: String,
    party: u64,
    /// Their party is open, and how many more people fit in it.
    open: bool,
    room: usize,
}

/// The times of an account's recent actions, those older than `window`
/// seconds dropped as of `now`.
fn recent(times: &mut VecDeque<f64>, now: f64, window: f64) {
    while times.front().is_some_and(|&t| now - t >= window) {
        times.pop_front();
    }
}

impl Server {
    /// A launcher player's friend action or RECORD.
    pub(super) fn social(&mut self, me: u64, message: ToServer, now: f64) {
        if let ToServer::Record(account) = message {
            return self.service_record(me, account, now);
        }
        if !self.friend_action_allowed(me, now) {
            return;
        }
        match message {
            ToServer::FriendRequest(gamertag) => self.friend_request(me, &gamertag, now),
            ToServer::FriendAccept(from) => self.friend_accept(me, from),
            ToServer::FriendDecline(from) => {
                let changed = self.friends.decline(me, from);
                self.friendship_changed(me, from, changed);
            }
            ToServer::FriendRemove(other) => {
                let changed = self.friends.remove(me, other);
                self.friendship_changed(me, other, changed);
            }
            _ => {}
        }
    }

    /// `account`'s friends list needs sending again, at the next pass.
    pub(super) fn friends_changed(&mut self, account: u64) {
        self.friends_marked.insert(account);
    }

    /// Whether `me` may make another friend action now: if not, they're
    /// told to slow down, once a minute.
    fn friend_action_allowed(&mut self, me: u64, now: f64) -> bool {
        let times = self.friend_actions.entry(me).or_default();
        recent(times, now, 60.0);
        if times.len() < FRIEND_ACTIONS_PER_MINUTE {
            times.push_back(now);
            return true;
        }
        if self.slowed.get(&me).is_none_or(|&t| now - t >= 60.0) {
            self.slowed.insert(me, now);
            self.notice(me, live::SLOW_DOWN);
        }
        false
    }

    /// Send `account` `message` if they're signed in on a launcher.
    fn tell_launcher(&mut self, account: u64, message: &ToPc) {
        if self.client_of(account) == ClientKind::Launcher {
            self.tell(account, message);
        }
    }

    fn gamertag_of(&self, account: u64) -> String {
        self.accounts
            .get(&account)
            .map_or(String::new(), |a| a.gamertag.clone())
    }

    /// `me` asks the player with `gamertag` to be friends.
    fn friend_request(&mut self, me: u64, gamertag: &str, now: f64) {
        let wanted = clean_name(gamertag);
        let named = |a: &&crate::store::Account| a.gamertag.eq_ignore_ascii_case(&wanted);
        let found = self.accounts.values().find(named);
        let Some((them, name)) = found
            .filter(|_| !wanted.is_empty())
            .map(|a| (a.id, a.gamertag.clone()))
        else {
            return self.notice(me, live::NO_SUCH_PLAYER);
        };
        let notice = |text: &str| ToPc::Notice(friend_notice(text, &name));
        match self.friends.ask(me, them, self.epoch + now as u64) {
            Ok(Asked::Sent) => {
                self.tell(me, &notice(live::FRIEND_ASKED));
                let asked = (me, them);
                let due = self
                    .asked_you
                    .get(&asked)
                    .is_none_or(|&t| now - t >= ASKED_YOU_EVERY);
                if due && self.client_of(them) == ClientKind::Launcher {
                    self.asked_you.insert(asked, now);
                    let mine = friend_notice(live::ASKED_YOU, &self.gamertag_of(me));
                    self.tell(them, &ToPc::Notice(mine));
                }
            }
            Ok(Asked::NowFriends) => {
                self.tell(me, &notice(live::NOW_FRIENDS));
                let mine = friend_notice(live::NOW_FRIENDS, &self.gamertag_of(me));
                self.tell_launcher(them, &ToPc::Notice(mine));
            }
            Err(refusal) => {
                let text = match refusal {
                    Refusal::Yourself => live::NOT_YOURSELF.to_string(),
                    Refusal::AlreadyFriends => friend_notice(live::ALREADY_FRIENDS, &name),
                    Refusal::AlreadyAsked => friend_notice(live::ALREADY_ASKED, &name),
                    Refusal::ListFull => live::LIST_FULL.to_string(),
                    Refusal::TheirRequestsFull => friend_notice(live::TOO_MANY_REQUESTS, &name),
                };
                return self.tell(me, &ToPc::Notice(text));
            }
        }
        self.friendship_changed(me, them, true);
    }

    /// `me` accepts `from`'s request.
    fn friend_accept(&mut self, me: u64, from: u64) {
        match self.friends.accept(me, from) {
            Ok(true) => {
                let accepted = friend_notice(live::ACCEPTED, &self.gamertag_of(me));
                self.tell_launcher(from, &ToPc::Notice(accepted));
                self.friendship_changed(me, from, true);
            }
            Ok(false) => self.friendship_changed(me, from, false),
            Err(_) => self.notice(me, live::LIST_FULL),
        }
    }

    /// What's between `me` and `other` changed (`changed`): both get new
    /// lists, and friends.txt is saved. If nothing changed, `me` acted on
    /// an old list: they're sent it again.
    fn friendship_changed(&mut self, me: u64, other: u64, changed: bool) {
        self.friends_changed(me);
        if !changed {
            if let Some(k) = self.pc_of(me) {
                self.pcs[k].friends_sent = None;
            }
            return;
        }
        self.friends_changed(other);
        let path = self.dir.join("friends.txt");
        if let Err(e) = self.friends.save(&path) {
            log(format_args!("live: can't save {}: {e}", path.display()));
        }
    }

    /// `me` asks for `account`'s service record.
    fn service_record(&mut self, me: u64, account: u64, now: f64) {
        let asks = self.record_asks.entry(me).or_default();
        recent(asks, now, 10.0);
        if asks.len() >= RECORDS_PER_10S {
            return;
        }
        asks.push_back(now);
        let client = self.client_of(me);
        let found = self.accounts.get(&account).map(|a| {
            let ranked = self
                .playlists
                .iter()
                .filter(|p| p.ranked && p.client == client);
            let playlists = ranked.filter_map(|p| {
                let s = a.stats(&p.key).filter(|s| s.games > 0)?;
                Some(PlaylistRecord {
                    playlist: p.id,
                    level: s.rank.level,
                    games: s.games,
                    wins: s.wins,
                    tally: s.tally.into(),
                })
            });
            Record {
                gamertag: a.gamertag.clone(),
                look: a.look,
                best: a.best_level(),
                created: a.created,
                playlists: playlists.take(MAX_RECORD_PLAYLISTS).collect(),
            }
        });
        self.tell(me, &ToPc::ServiceRecord(ServiceRecord { account, found }));
    }

    /// Once a second at most: work out everyone's presence, mark the lists
    /// that changed, and send each launcher player marked their list (if it
    /// isn't the one they have).
    pub(super) fn send_friends(&mut self, now: f64) {
        if now - self.friends_passed < FRIENDS_EVERY {
            return;
        }
        self.friends_passed = now;
        self.forget_old_actions(now);
        let mut sizes: HashMap<u64, usize> = HashMap::new();
        for pc in self.pcs.iter().filter(|pc| pc.account.is_some()) {
            *sizes.entry(pc.party).or_default() += 1 + usize::from(pc.guests);
        }
        // One pass over the PCs signed in.
        let mut pcs: HashMap<u64, usize> = HashMap::new();
        let mut presences: HashMap<u64, Presence> = HashMap::new();
        for (k, pc) in self.pcs.iter().enumerate() {
            let Some(id) = pc.account.filter(|_| pc.gone.is_none()) else {
                continue;
            };
            pcs.insert(id, k);
            if let Some(p) = self.presence(id, pc, &sizes) {
                presences.insert(id, p);
            }
        }
        // Who changed, signed in or signed out: they and their friends.
        let mut changed: Vec<u64> = presences
            .iter()
            .filter(|(id, p)| self.presences.get(id) != Some(p))
            .map(|(&id, _)| id)
            .collect();
        changed.extend(
            self.presences
                .keys()
                .filter(|id| !presences.contains_key(id)),
        );
        // (Requests either way show their gamertag and level too.)
        for id in changed {
            self.friends_changed(id);
            let mut theirs = self.friends.friends_of(id);
            theirs.extend(self.friends.asked_by(id));
            theirs.extend(self.friends.asking(id));
            for other in theirs {
                self.friends_changed(other);
            }
        }
        // (Those who signed out are passed on now, and dropped.)
        self.presences = presences;
        for id in std::mem::take(&mut self.friends_marked) {
            let Some(&k) = pcs.get(&id) else {
                continue;
            };
            if self.pcs[k].client != ClientKind::Launcher {
                continue;
            }
            let (kind, body) = ToPc::Friends(self.friends_list(id)).write();
            let pc = &mut self.pcs[k];
            if pc.friends_sent.as_ref() != Some(&body) {
                pc.conn.send(kind, &body);
                pc.friends_sent = Some(body);
            }
        }
    }

    /// Drop the times of friend actions, RECORDs, slow-down notices and
    /// ASKED_YOUs too old to matter, so they don't pile up.
    fn forget_old_actions(&mut self, now: f64) {
        self.friend_actions.retain(|_, times| {
            recent(times, now, 60.0);
            !times.is_empty()
        });
        self.record_asks.retain(|_, times| {
            recent(times, now, 10.0);
            !times.is_empty()
        });
        self.slowed.retain(|_, t| now - *t < 60.0);
        self.asked_you.retain(|_, t| now - *t < ASKED_YOU_EVERY);
    }

    /// How `id`, signed in on `pc`, looks to their friends (`sizes`: the
    /// people in each party).
    fn presence(&self, id: u64, pc: &Pc, sizes: &HashMap<u64, usize>) -> Option<Presence> {
        let account = self.accounts.get(&id)?;
        let mut p = Presence {
            client: pc.client,
            gamertag: account.gamertag.clone(),
            best: account.best_level(),
            activity: Activity::Lobby,
            playlist: 0,
            map: String::new(),
            variant: String::new(),
            party: 0,
            open: false,
            room: 0,
        };
        let party = self.parties.get(&pc.party);
        if let (ClientKind::Launcher, Some(party)) = (pc.client, party) {
            let size = sizes.get(&pc.party).copied().unwrap_or(1);
            let playing = self
                .playing(id)
                .filter(|_| party.activity == Activity::Playing);
            let (map, variant) = playing.unwrap_or_default();
            p.map = map.to_string();
            p.variant = variant.to_string();
            p.activity = party.activity;
            p.playlist = party.playlist;
            p.party = pc.party;
            p.open = party.privacy == Privacy::Open;
            p.room = MAX_PARTY.saturating_sub(size);
        }
        Some(p)
    }

    /// `me`'s friends list as it is now: requests to them (oldest first),
    /// friends (by gamertag, whatever the case), then requests they sent
    /// (oldest first). Accounts the server doesn't have are left out.
    fn friends_list(&self, me: u64) -> Vec<Friend> {
        let entry = |account: u64, relation: Relation| {
            let a = self.accounts.get(&account)?;
            let mut f = Friend {
                account,
                gamertag: a.gamertag.clone(),
                relation,
                best: a.best_level(),
                online: Online::Offline,
                activity: Activity::Lobby,
                playlist: 0,
                map: String::new(),
                variant: String::new(),
                party: 0,
                joinable: false,
            };
            // Only friends show where they are.
            let presence = self.presences.get(&account);
            match presence.filter(|_| relation == Relation::Friend) {
                Some(p) if p.client == ClientKind::Launcher => {
                    f.online = Online::Launcher;
                    f.activity = p.activity;
                    f.playlist = p.playlist;
                    f.map = p.map.clone();
                    f.variant = p.variant.clone();
                    f.party = p.party;
                    f.joinable = self.can_join(me, p.party).is_ok();
                }
                Some(_) => f.online = Online::Game,
                None => {}
            }
            Some(f)
        };
        let asked_us = self.friends.asked_by(me).into_iter();
        let mut list: Vec<Friend> = asked_us
            .filter_map(|a| entry(a, Relation::AskedUs))
            .collect();
        let mut friends: Vec<Friend> = (self.friends.friends_of(me).into_iter())
            .filter_map(|a| entry(a, Relation::Friend))
            .collect();
        friends.sort_by_cached_key(|f| (f.gamertag.to_ascii_lowercase(), f.account));
        list.extend(friends);
        let we_asked = self.friends.asking(me).into_iter();
        list.extend(we_asked.filter_map(|a| entry(a, Relation::WeAsked)));
        list.truncate(MAX_FRIEND_ENTRIES);
        list
    }
}
