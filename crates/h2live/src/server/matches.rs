//! Matches, from the search to the levels won and lost. A party's leader
//! searches a playlist; the matchmaker puts parties together, and everyone
//! in the match hears of it (MATCH) while the PC with the best connection
//! is asked to host it (HOST_MATCH). Once that PC says it is (HOSTING),
//! every other PC is linked to it through the relay (LINK), and the game
//! starts (GO) when all have come, or 20 seconds on: a PC that hasn't come
//! by then is left out of it. When the game ends each PC says how it saw it
//! end (RESULT). If at least half of the joined PCs that answer within 30
//! seconds of the end (the host's result, or half of theirs) saw it as the
//! host did, levels change as `levels` says, and everyone hears how, theirs
//! and everyone else's (MATCH_OVER). The host's word alone isn't enough:
//! unless every joined PC left the game, at least one must have answered.
//!
//! A host that leaves ends the match: it doesn't count for anyone, except
//! as a loss for the host if it quit or its link to the server died. Joined
//! PCs saying they lost the host end it too, if at least half of them say
//! so and the relay saw it go (a loss for the host too if its link to the
//! server had gone quiet as well: its PC or its network died). A player
//! who quits a game in progress (or says it lost a host that was there,
//! one that gave up on it when it went quiet, say) loses it as `levels`
//! says: last in free-for-all, with their team otherwise. Leaving once
//! the game is over (after one's own result, or the host's) isn't
//! quitting. A game that runs five minutes past its time limit is given
//! up on.
//!
//! Parties play custom games too: the leader hosts on a map everyone has,
//! and every member is linked to them, those who join later as well. They
//! change no levels.
//!
//! The launcher's matches (from its playlists) go the same way, on MCC's
//! engine, with the engine's packets going through the UDP relay rather
//! than relay legs. When one forms, the server opens a room on the relay
//! for it (the match's id) and tells each player in it (LAUNCHER_MATCH, in
//! place of MATCH) the MCC map and game variant, everyone in it, who hosts,
//! and their own way into the room: the relay's port, the room, their id
//! in it (`relay_id`: the same for their account in every match, and their
//! XUID in the engine) and the key for that id, which only they are given.
//! The host is asked to host (HOST_MATCH) and says when it is (HOSTING),
//! as in the game; each other launcher says when its game joined the
//! host's (JOINED), and the game starts (GO) once all have, or 20 seconds
//! after HOSTING without those that haven't. At the end each launcher
//! sends the engine's results (LAUNCHER_RESULT), which count as RESULTs
//! do: the host's, confirmed by the others, and only if the host's says
//! the game finished. Levels change the same way. The room closes when
//! the match is over, however it ends. A host lost to the others ends the
//! match once its link to the server goes too (the relay can't say it saw
//! it go).
//!
//! A launcher party's leader starts custom games (LAUNCHER_CUSTOM) with an
//! MCC variant from the launcher playlists and a map every member has that
//! they play. Each is a match of its own, made here rather than by the
//! matchmaker: the leader hosts it, the others join, teams alternate down
//! the party, and it changes no levels. The party is back in its lobby when
//! it's over, for the leader to start another.

use super::party::OTHER_PROGRAM;
use super::relay::QUIET;
use super::{log, Server};
use crate::card;
use crate::levels::{self, Finish, Placed, Rank, MAX_LEVEL};
use crate::matchmaker::{self, Event, Member, Seat, Ticket};
use crate::store::{self, Counted, GameRecord, RecordedPlayer, Stats};
use h2net::live::{
    Activity, ClientKind, LauncherMatch, LauncherPlayer, LauncherResult, MatchInfo, MatchOver,
    MatchPlayer, PlayerResult, RelaySeat, SearchStatus, Stage, ToPc, CUSTOM_GAME, QUICKMATCH,
};
use h2net::ANY_TEAM;

/// Seconds from HOSTING to GO at the latest, for PCs slow to link.
const LINK_WAIT: f64 = 20.0;
/// Seconds after the game ends (see `Match::over_by_results`) that
/// everyone has to send their results.
const RESULTS_WAIT: f64 = 30.0;
/// Seconds a launcher match's relay room stays open after the match ends:
/// the engines still talk through their postgame (about 7 s seen on the
/// owner's PC), and a joining PC that loses the host there waits out the
/// engine's 45 s timeout before it leaves. An estimate with room to spare.
const ROOM_GRACE: f64 = 30.0;
/// Seconds after GO that a game is given up on: this long past its time
/// limit, or this long if it has none.
const OVERTIME: f64 = 300.0;
const NO_TIME_LIMIT: f64 = 3600.0;
/// PLAYLISTS go at most this often (seconds) as the counts in them change.
const PLAYLISTS_EVERY: f64 = 5.0;
/// A PC whose round trips to the server aren't known yet counts as this
/// slow (milliseconds).
const UNKNOWN_ROUND_TRIP: u32 = 1000;

/// What players are told.
const PARTY_CHANGED: &str = "YOUR PARTY CHANGED. SEARCH AGAIN.";
const STILL_PLAYING: &str = "SOMEONE IN YOUR PARTY IS STILL IN A GAME";
const NO_MAPS: &str = "YOUR PARTY HAS NO MAP IN COMMON";
/// Why a match didn't count (MATCH_OVER).
const HOST_LEFT: &str = "THE HOST LEFT. THE GAME DIDN'T COUNT.";
const HOST_QUIT: &str = "YOU QUIT AS HOST. IT COUNTS AS A LOSS.";
const DISPUTED: &str = "THE RESULTS DIDN'T AGREE. THE GAME DIDN'T COUNT.";
const NO_RESULT: &str = "THE HOST SENT NO RESULT. THE GAME DIDN'T COUNT.";
const UNCONFIRMED: &str = "NO ONE CONFIRMED THE RESULT. THE GAME DIDN'T COUNT.";
const NEVER_ENDED: &str = "THE GAME NEVER ENDED. IT DIDN'T COUNT.";
const NO_OPPONENTS: &str = "NO ONE PLAYED AGAINST YOU. THE GAME DIDN'T COUNT.";
const NO_HOST: &str = "NO ONE COULD HOST THE GAME.";
const NOT_JOINED: &str = "YOU DIDN'T JOIN THE GAME.";
const UNFINISHED: &str = "THE GAME DIDN'T FINISH. IT DIDN'T COUNT.";
/// Launchers' matches need the relay.
const NO_RELAY: &str = "THIS SERVER HAS NO RELAY FOR ONLINE GAMES";
const NO_CUSTOM_GAMES: &str = "CUSTOM GAMES AREN'T READY ON THE LAUNCHER YET";
const NO_SUCH_GAME: &str = "THAT GAME TYPE ISN'T ON THIS SERVER";
const NO_SUCH_MAP: &str = "NOT EVERYONE IN YOUR PARTY HAS THAT MAP";

/// A player's id on the relay, and so their XUID in MCC's engine: their
/// account (the same in every match), but for the two the relay keeps
/// for itself (0, which the relay server sends from, and its broadcast id).
pub fn relay_id(account: u64) -> u64 {
    match account {
        0 => 1,
        u64::MAX => u64::MAX - 1,
        account => account,
    }
}

/// What a launcher's match has beyond the game's.
pub(super) struct Launcher {
    /// The MCC game variant it plays.
    variant: String,
    /// Its room on the relay.
    room: u64,
    /// What each launcher's LAUNCHER_RESULT said beyond its players'
    /// results: whether the game finished, and the team scores.
    reports: Vec<(u64, bool, Vec<i32>)>,
}

/// A match from the moment it formed until everyone hears how it went.
pub(super) struct Match {
    /// As the players were told (MATCH).
    pub(super) info: MatchInfo,
    /// Every PC in it, as the matchmaker seated them.
    pub(super) seats: Vec<Seat>,
    /// When the host said it's hosting, and when the game started (GO).
    hosting: Option<f64>,
    started: Option<f64>,
    /// Joining PCs whose link to the host came.
    linked: Vec<u64>,
    /// PCs out of it before the game started, who were told it didn't
    /// count for them.
    pub(super) out: Vec<u64>,
    /// PCs that quit the game in progress, and joining PCs that said they
    /// lost the host.
    quit: Vec<u64>,
    lost_host: Vec<u64>,
    /// PCs that left once the game was over for them (they or the host had
    /// said how it ended): they're rated as the results say.
    left_at_end: Vec<u64>,
    /// Joining PCs whose relay link to the host broke as the host went
    /// (see `Server::relay`).
    host_dropped: Vec<u64>,
    /// The host quit, through its menu or because its link to the server
    /// died (or went quiet as the joined PCs lost it).
    host_quit: bool,
    /// Why it can't count for anyone, if it can't.
    void: Option<&'static str>,
    /// Each PC's RESULT, and when the game ended (as the host, or half
    /// the joining PCs still in it, said).
    results: Vec<(u64, Vec<PlayerResult>)>,
    ended: Option<f64>,
    /// For a launcher's match, its room on the relay and the rest.
    launcher: Option<Launcher>,
}

impl Match {
    fn new(info: MatchInfo, seats: Vec<Seat>) -> Match {
        Match {
            info,
            seats,
            hosting: None,
            started: None,
            linked: Vec::new(),
            out: Vec::new(),
            quit: Vec::new(),
            lost_host: Vec::new(),
            left_at_end: Vec::new(),
            host_dropped: Vec::new(),
            host_quit: false,
            void: None,
            results: Vec::new(),
            ended: None,
            launcher: None,
        }
    }

    /// `account` is in it (it may have quit the game: then it still loses).
    fn has(&self, account: u64) -> bool {
        self.seats.iter().any(|s| s.account == account) && !self.out.contains(&account)
    }

    /// The PCs in it other than the host.
    fn joiners(&self) -> impl Iterator<Item = u64> + '_ {
        let accounts = self.seats.iter().map(|s| s.account);
        accounts.filter(|&a| a != self.info.host && !self.out.contains(&a))
    }

    fn reported(&self, account: u64) -> bool {
        self.results.iter().any(|(a, _)| *a == account)
    }

    /// `account` left the game in progress: it quit, or said it lost the
    /// host.
    fn left(&self, account: u64) -> bool {
        self.quit.contains(&account) || self.lost_host.contains(&account)
    }

    /// `account` left the game, before its end or after.
    fn gone(&self, account: u64) -> bool {
        self.left(account) || self.left_at_end.contains(&account)
    }

    /// The game is over, as the host or at least half the joining PCs
    /// still in it say. (One PC's word isn't enough to end it for the
    /// others.)
    fn over_by_results(&self) -> bool {
        let staying: Vec<u64> = self.joiners().filter(|&a| !self.left(a)).collect();
        let reported = staying.iter().filter(|&&a| self.reported(a)).count();
        self.reported(self.info.host) || !staying.is_empty() && reported * 2 >= staying.len()
    }

    /// Everyone in the game has had their say: a result, or they're gone.
    fn all_in(&self) -> bool {
        (self.host_quit || self.reported(self.info.host))
            && self.joiners().all(|a| self.reported(a) || self.gone(a))
    }

    /// At least half the joining PCs lost the host, and their links to it
    /// broke on its side: it's gone.
    fn lost_the_host(&self) -> bool {
        let joiners = self.joiners().count();
        let lost = self
            .lost_host
            .iter()
            .filter(|a| self.host_dropped.contains(a));
        self.started.is_some() && joiners > 0 && lost.count() * 2 >= joiners
    }

    /// The game has run too long by `now`.
    fn too_long(&self, now: f64) -> bool {
        let longest = match self.info.time_limit {
            0 => NO_TIME_LIMIT,
            limit => f64::from(limit) + OVERTIME,
        };
        self.started.is_some_and(|t| now >= t + longest)
    }
}

/// Whether a joined PC saw the game end as the host did: everyone on the
/// same team, in the same place.
fn agrees(host: &[PlayerResult], theirs: &[PlayerResult]) -> bool {
    let order = |results: &[PlayerResult]| {
        let mut order: Vec<(u64, u8, u8)> = results
            .iter()
            .map(|r| (r.account, r.team, r.place))
            .collect();
        order.sort_unstable();
        order
    };
    order(host) == order(theirs)
}

/// A search's stage as STATUS sends it.
fn stage(stage: matchmaker::Stage) -> Stage {
    match stage {
        matchmaker::Stage::Searching => Stage::Searching,
        matchmaker::Stage::Gathering => Stage::Gathering,
        matchmaker::Stage::WaitingToFill => Stage::WaitingToFill,
        matchmaker::Stage::Balancing => Stage::Balancing,
    }
}

impl Server {
    /// The accounts in party `id`.
    fn members_of(&self, id: u64) -> Vec<u64> {
        self.parties
            .get(&id)
            .map_or(Vec::new(), |p| p.members.clone())
    }

    /// Tell everyone in party `id` `message`.
    fn tell_party(&mut self, id: u64, message: &ToPc) {
        for account in self.members_of(id) {
            self.tell(account, message);
        }
    }

    /// `account` is in a match that isn't over for it: about to start, or
    /// playing, or waiting for the others' results.
    fn busy(&self, account: u64) -> bool {
        self.matches
            .iter()
            .any(|m| m.has(account) && !m.gone(account))
    }

    /// `me`, leading their party, starts it searching `playlist` (or
    /// `QUICKMATCH`).
    pub(super) fn search(&mut self, me: u64, playlist: u8, now: f64) {
        let Some((id, party)) = self.led_by(me) else {
            return;
        };
        if !matches!(party.activity, Activity::Lobby | Activity::Searching) {
            return self.notice(me, STILL_PLAYING);
        }
        let (members, previous_map) = (party.members.clone(), party.previous_map.clone());
        if members.iter().any(|&a| self.busy(a)) {
            return self.notice(me, STILL_PLAYING);
        }
        let client = self.client_of(me);
        if members.iter().any(|&a| self.client_of(a) != client) {
            return self.notice(me, OTHER_PROGRAM);
        }
        if client == ClientKind::Launcher && self.relay.is_none() {
            return self.notice(me, NO_RELAY);
        }
        let members = members.iter().filter_map(|&a| {
            let pc = &self.pcs[self.pc_of(a)?];
            let account = self.accounts.get(&a)?;
            let ranked = self
                .playlists
                .iter()
                .filter(|p| p.ranked && p.client == client);
            let levels = ranked.filter_map(|p| Some((p.id, account.stats(&p.key)?.rank.level)));
            Some(Member {
                account: a,
                levels: levels.collect(),
                guests: pc.guests,
                maps: pc.maps.clone(),
                rtt: self.round_trip(a).unwrap_or(UNKNOWN_ROUND_TRIP),
            })
        });
        let ticket = Ticket {
            party: id,
            client,
            playlist,
            members: members.collect(),
            previous_map,
        };
        let searching = self.matchmaker.search(ticket, now);
        let Some(party) = self.parties.get_mut(&id) else {
            return;
        };
        match searching {
            Ok(playlist) => {
                party.activity = Activity::Searching;
                party.playlist = playlist;
            }
            Err(why) => {
                party.activity = Activity::Lobby;
                party.playlist = QUICKMATCH;
                self.notice(me, &why.to_uppercase());
            }
        }
        self.party_changed(id);
    }

    /// `me`, leading their party, stops its search.
    pub(super) fn cancel(&mut self, me: u64) {
        if let Some((id, party)) = self.led_by(me) {
            if party.activity == Activity::Searching {
                self.stop_search(id, None);
            }
        }
    }

    /// Stop party `id` searching, telling its members `why` if there's a
    /// reason to.
    pub(super) fn stop_search(&mut self, id: u64, why: Option<&str>) {
        self.matchmaker.cancel(id);
        if let Some(party) = self.parties.get_mut(&id) {
            party.activity = Activity::Lobby;
            party.playlist = QUICKMATCH;
        }
        if let Some(why) = why {
            self.tell_party(id, &ToPc::Notice(why.into()));
        }
        self.party_changed(id);
    }

    /// Party `id` changed while searching: it has to search again.
    pub(super) fn party_changed_searching(&mut self, id: u64) {
        let searching = self.parties.get(&id).map(|p| p.activity) == Some(Activity::Searching);
        if searching {
            self.stop_search(id, Some(PARTY_CHANGED));
        }
    }

    /// Search, and act on what the matchmaker found.
    pub(super) fn matchmake(&mut self, now: f64) {
        for event in self.matchmaker.poll(now) {
            self.matchmaker_event(event, now);
        }
    }

    fn matchmaker_event(&mut self, event: Event, now: f64) {
        match event {
            Event::Status { party, status } => {
                let status = SearchStatus {
                    stage: stage(status.stage),
                    have: status.have,
                    need: status.need,
                    seconds: status.seconds,
                    low: status.low,
                    high: status.high,
                };
                self.tell_party(party, &ToPc::Status(status));
            }
            Event::Formed(m) => self.formed(m),
            Event::Failed { party } => {
                let failed = SearchStatus {
                    stage: Stage::Failed,
                    have: 0,
                    need: 0,
                    seconds: 0,
                    low: 1,
                    high: MAX_LEVEL,
                };
                self.tell_party(party, &ToPc::Status(failed));
                self.stop_search(party, None);
            }
            Event::NewHost { id, host } => {
                let Some(k) = self.matches.iter().position(|m| m.info.id == id) else {
                    return;
                };
                if !self.matches[k].has(host) {
                    // Gone already: ask the next.
                    let next = self.matchmaker.host_failed(id, now);
                    return self.matchmaker_event(next.unwrap_or(Event::NoHost { id }), now);
                }
                // Everyone hears who hosts now. Launchers that joined the
                // last one's game join the new host's afresh.
                let m = &mut self.matches[k];
                m.info.host = host;
                m.linked.clear();
                self.issue_room(k);
                let players: Vec<u64> = self.matches[k].seats.iter().map(|s| s.account).collect();
                for account in players {
                    if self.matches[k].has(account) {
                        self.tell_match(k, account);
                    }
                }
                self.tell(host, &ToPc::HostMatch(id));
            }
            Event::NoHost { id } => {
                if let Some(m) = self.matches.iter_mut().find(|m| m.info.id == id) {
                    m.void = Some(NO_HOST);
                }
            }
        }
    }

    /// Tell `account` about match `k` (MATCH, or LAUNCHER_MATCH with their
    /// own way into its room on the relay).
    fn tell_match(&mut self, k: usize, account: u64) {
        let m = &self.matches[k];
        let message = match &m.launcher {
            None => Some(ToPc::Match(m.info.clone())),
            Some(launcher) => self.launcher_match(m, launcher, account),
        };
        if let Some(message) = message {
            self.tell(account, &message);
        }
    }

    /// Launcher match `m` as `account` is told it, if the relay is on.
    fn launcher_match(&self, m: &Match, launcher: &Launcher, account: u64) -> Option<ToPc> {
        let relay = self.relay.as_ref()?;
        let id = relay_id(account);
        let players = m.info.players.iter().map(|p| LauncherPlayer {
            relay_id: relay_id(p.account),
            account: p.account,
            gamertag: p.gamertag.clone(),
            team: p.team,
            level: p.level,
            party: p.party,
        });
        Some(ToPc::LauncherMatch(LauncherMatch {
            id: m.info.id,
            playlist: m.info.playlist,
            ranked: m.info.ranked,
            teams: m.info.game_type.teams(),
            map: m.info.map.clone(),
            variant: launcher.variant.clone(),
            relay: RelaySeat {
                port: relay.local_addr().port(),
                room: launcher.room,
                id,
                key: relay.member_key(launcher.room, id).0,
            },
            host: relay_id(m.info.host),
            countdown: m.info.countdown,
            players: players.collect(),
        }))
    }

    /// Open match `k`'s room on the relay, if it's a launcher's, or keep
    /// it open: an empty room lasts a minute, and this starts that over.
    fn issue_room(&self, k: usize) {
        let room = self.matches[k].launcher.as_ref().map(|l| l.room);
        if let (Some(room), Some(relay)) = (room, &self.relay) {
            relay.issue(room);
        }
    }

    /// The matchmaker made match `m`: tell everyone in it, and ask its host
    /// to host. A launcher's gets a room on the relay first.
    fn formed(&mut self, m: matchmaker::Match) {
        let players = m.players.iter().map(|s| {
            let account = self.accounts.get(&s.account);
            MatchPlayer {
                account: s.account,
                gamertag: account.map_or(String::new(), |a| a.gamertag.clone()),
                look: account.map_or_else(Default::default, |a| a.look),
                level: s.level,
                team: s.team,
                party: s.party,
                guests: s.guests,
            }
        });
        let info = MatchInfo {
            id: m.id,
            playlist: m.playlist,
            ranked: m.ranked,
            map: m.map.clone(),
            hash: m.hash,
            game_type: m.variant.game_type,
            preset: m.variant.preset.clone(),
            score: m.variant.score,
            time_limit: m.variant.time_limit,
            bots: m.bots,
            host: m.host,
            countdown: LINK_WAIT as u8,
            players: players.collect(),
        };
        let launcher = m.variant.mcc.clone().map(|variant| Launcher {
            variant,
            room: m.id,
            reports: Vec::new(),
        });
        let game = match &launcher {
            None => format!("{:?}", m.variant.game_type),
            Some(l) => format!("{} ({:?}) for launchers", l.variant, m.variant.game_type),
        };
        log(format_args!(
            "live: match {:016x}: {game} on {}, {} PCs and {} bots",
            m.id,
            m.map,
            m.players.len(),
            m.bots
        ));
        for s in &m.players {
            if let Some(party) = self.parties.get_mut(&s.party) {
                party.activity = Activity::Playing;
                party.playlist = m.playlist;
            }
            self.party_changed(s.party);
        }
        let people: usize = m.players.iter().map(|s| 1 + usize::from(s.guests)).sum();
        let levels = m.players.iter().map(|s| s.level);
        let joining = SearchStatus {
            stage: Stage::Joining,
            have: people as u8,
            need: 0,
            seconds: 0,
            low: levels.clone().min().unwrap_or(1),
            high: levels.max().unwrap_or(MAX_LEVEL),
        };
        let mut record = Match::new(info, m.players.clone());
        let no_relay = launcher.is_some() && self.relay.is_none();
        record.launcher = launcher;
        if no_relay {
            // (Searches are refused without it, so this never happens.)
            record.void = Some(NO_RELAY);
        }
        self.relayed.insert(m.id, 0);
        self.matches.push(record);
        if no_relay {
            return;
        }
        let k = self.matches.len() - 1;
        // Before anyone has the keys to it.
        self.issue_room(k);
        for s in &m.players {
            self.tell_match(k, s.account);
            self.tell(s.account, &ToPc::Status(joining));
        }
        self.tell(m.host, &ToPc::HostMatch(m.id));
    }

    /// `me` hosts match `id` now: link everyone else in it to them.
    pub(super) fn hosting(&mut self, me: u64, id: u64, now: f64) {
        let Some(k) = self.matches.iter().position(|m| m.info.id == id) else {
            return;
        };
        if self.matches[k].hosting.is_some() || !self.matchmaker.hosting(id, me) {
            return;
        }
        let m = &mut self.matches[k];
        m.hosting = Some(now);
        let map = m.info.map.clone();
        let launcher = m.launcher.is_some();
        let seat = |a: u64| m.seats.iter().find(|s| s.account == a).copied();
        let Some(host) = seat(me) else {
            return;
        };
        let joiners: Vec<Seat> = m.joiners().filter_map(seat).collect();
        let starting = SearchStatus {
            stage: Stage::Starting,
            have: 0,
            need: 0,
            seconds: LINK_WAIT as u16,
            low: 1,
            high: MAX_LEVEL,
        };
        // Launchers join the host's game through the relay by themselves.
        self.issue_room(k);
        self.tell(me, &ToPc::Status(starting));
        for j in joiners {
            if !launcher {
                self.link(
                    id,
                    &map,
                    (me, host.level, host.team),
                    (j.account, j.level, j.team),
                );
            }
            self.tell(j.account, &ToPc::Status(starting));
        }
    }

    /// A launcher in match `id` says its game joined the host's game.
    pub(super) fn joined(&mut self, me: u64, id: u64) {
        let theirs = |m: &&mut Match| m.info.id == id && m.launcher.is_some();
        if let Some(m) = self.matches.iter_mut().find(theirs) {
            if m.info.host != me {
                self.linked(id, me);
            }
        }
    }

    /// A joining PC's link to the host of match `id` came.
    pub(super) fn linked(&mut self, id: u64, joiner: u64) {
        if let Some(m) = self.matches.iter_mut().find(|m| m.info.id == id) {
            if m.has(joiner) && !m.linked.contains(&joiner) {
                m.linked.push(joiner);
            }
        }
    }

    /// `me` says how the game's match `id` ended (the first time only, and
    /// only if it was there at the end).
    pub(super) fn result(&mut self, me: u64, id: u64, players: Vec<PlayerResult>) {
        let theirs = |m: &&mut Match| m.info.id == id && m.launcher.is_none();
        if let Some(m) = self.matches.iter_mut().find(theirs) {
            if m.started.is_some() && m.has(me) && !m.reported(me) && !m.gone(me) {
                m.results.push((me, players));
            }
        }
    }

    /// `me` says how launcher match `id` ended, as RESULT does, naming its
    /// players by their relay ids (those not in the match left out).
    pub(super) fn launcher_result(&mut self, me: u64, result: LauncherResult) {
        let id = result.id;
        let theirs = |m: &&mut Match| m.info.id == id && m.launcher.is_some();
        let Some(m) = self.matches.iter_mut().find(theirs) else {
            return;
        };
        if m.started.is_none() || !m.has(me) || m.reported(me) || m.gone(me) {
            return;
        }
        let players = result.players.iter().filter_map(|p| {
            let seat = m.seats.iter().find(|s| relay_id(s.account) == p.relay_id)?;
            Some(PlayerResult {
                account: seat.account,
                team: p.team,
                place: p.place,
                score: p.score,
                kills: p.kills,
                deaths: p.deaths,
                left: p.left,
            })
        });
        m.results.push((me, players.collect()));
        if let Some(launcher) = &mut m.launcher {
            launcher
                .reports
                .push((me, result.finished, result.team_scores));
        }
    }

    /// `me` left match `id`: it quit, or lost the host.
    pub(super) fn left_match(&mut self, me: u64, id: u64, host_lost: bool) {
        if let Some(k) = self.matches.iter().position(|m| m.info.id == id) {
            if self.matches[k].has(me) && !self.matches[k].gone(me) {
                self.leave(k, me, host_lost);
            }
        }
    }

    /// The relay link from `joiner` to the host of match `id` broke on the
    /// host's side.
    pub(super) fn host_dropped(&mut self, id: u64, joiner: u64) {
        if let Some(m) = self.matches.iter_mut().find(|m| m.info.id == id) {
            m.host_dropped.push(joiner);
        }
    }

    /// `account`'s link to the server died: it quit whatever match it was
    /// in.
    pub(super) fn gone(&mut self, account: u64) {
        for k in 0..self.matches.len() {
            let m = &self.matches[k];
            if m.has(account) && !m.gone(account) {
                self.leave(k, account, false);
            }
        }
    }

    /// `account` left match `k`, having lost the host if `host_lost`.
    fn leave(&mut self, k: usize, account: u64, host_lost: bool) {
        let m = &mut self.matches[k];
        if m.reported(account) || m.reported(m.info.host) {
            // The game was over: what it's worth is up to the results.
            m.left_at_end.push(account);
        } else if account == m.info.host && m.hosting.is_some() {
            m.host_quit = true;
        } else if m.started.is_none() {
            // It's out, unrated. (If it was asked to host, the next is
            // asked; see `run_matches`.)
            self.leave_early(k, account);
        } else if host_lost {
            m.lost_host.push(account);
        } else {
            m.quit.push(account);
        }
        self.free_party(account);
    }

    /// `account` is out of match `k` before its game started: it didn't
    /// count for them.
    fn leave_early(&mut self, k: usize, account: u64) {
        let m = &mut self.matches[k];
        m.out.push(account);
        let id = m.info.id;
        self.unlink(id, Some(account));
        let over = MatchOver {
            id,
            counted: false,
            reason: NOT_JOINED.into(),
            card: self.card_of(account),
            levels: Vec::new(),
            players: Vec::new(),
        };
        self.tell(account, &ToPc::MatchOver(over));
        self.free_party(account);
    }

    /// `account`'s PC hasn't been heard from for a while, or is gone.
    fn quiet(&self, account: u64, now: f64) -> bool {
        self.pc_of(account)
            .is_none_or(|k| now - self.pcs[k].heard >= QUIET)
    }

    fn card_of(&self, account: u64) -> String {
        self.accounts
            .get(&account)
            .map_or(String::new(), |a| card::sign(a, &self.key))
    }

    /// `account`'s party goes back to its lobby, if it was playing a match
    /// and none of its members is any more. It keeps the match's playlist,
    /// so its members show their new level in it.
    fn free_party(&mut self, account: u64) {
        let Some(k) = self.pc_of(account) else {
            return;
        };
        let id = self.pcs[k].party;
        let members = self.members_of(id);
        let playing = self.parties.get(&id).map(|p| p.activity) == Some(Activity::Playing);
        if playing && !members.iter().any(|&a| self.busy(a)) {
            if let Some(party) = self.parties.get_mut(&id) {
                party.activity = Activity::Lobby;
            }
            self.party_changed(id);
        }
    }

    /// Move matches along: ask another PC to host if the one asked left,
    /// start games, and end matches whose results are in (or can't come).
    pub(super) fn run_matches(&mut self, now: f64) {
        self.close_rooms(now);
        let mut k = 0;
        while k < self.matches.len() {
            let m = &mut self.matches[k];
            let id = m.info.id;
            if m.ended.is_none() && m.over_by_results() {
                m.ended = Some(now);
            }
            if m.void.is_none() && m.too_long(now) {
                m.void = Some(NEVER_ENDED);
            }
            if m.hosting.is_none() && m.void.is_none() && m.out.contains(&m.info.host) {
                let next = self.matchmaker.host_failed(id, now);
                self.matchmaker_event(next.unwrap_or(Event::NoHost { id }), now);
                continue;
            }
            let m = &self.matches[k];
            let all_linked = m.joiners().all(|a| m.linked.contains(&a));
            if m.started.is_none()
                && m.hosting
                    .is_some_and(|t| all_linked || now >= t + LINK_WAIT)
            {
                self.go(k, now);
            }
            let m = &self.matches[k];
            let over = m.void.is_some()
                || m.host_quit
                || m.lost_the_host()
                || m.started.is_some() && m.all_in()
                || m.ended.is_some_and(|t| now >= t + RESULTS_WAIT);
            if over {
                self.end(k, now);
            } else {
                k += 1;
            }
        }
    }

    /// Close the relay rooms of ended launcher matches whose grace is up.
    fn close_rooms(&mut self, now: f64) {
        let relay = &self.relay;
        self.closing_rooms.retain(|&(room, at)| {
            let due = now >= at;
            if let (true, Some(relay)) = (due, relay) {
                relay.revoke(room);
            }
            !due
        });
    }

    /// Start match `k`'s game, without PCs that aren't linked to the host.
    fn go(&mut self, k: usize, now: f64) {
        let m = &self.matches[k];
        let late: Vec<u64> = m.joiners().filter(|a| !m.linked.contains(a)).collect();
        for account in late {
            self.leave_early(k, account);
        }
        let m = &mut self.matches[k];
        m.started = Some(now);
        let (id, players) = (m.info.id, m.seats.clone());
        let pcs = 1 + m.linked.len();
        log(format_args!("live: match {id:016x} started with {pcs} PCs"));
        for s in players {
            if self.matches[k].has(s.account) {
                self.tell(s.account, &ToPc::Go(id));
            }
        }
    }

    /// Match `k` is over: work out what it was worth to each player, keep
    /// it, and tell them.
    fn end(&mut self, k: usize, now: f64) {
        let mut m = self.matches.remove(k);
        // A host lost to the joined PCs and quiet on its link to the server
        // too has gone (its PC or its network died): as if that link had
        // died, though the server has yet to give up on it.
        if m.lost_the_host() && self.quiet(m.info.host, now) {
            m.host_quit = true;
        }
        let (id, host, teams) = (m.info.id, m.info.host, m.info.game_type.teams());
        // A launcher's host can say its game was cut short.
        let host_report = m.launcher.as_ref().and_then(|l| {
            let report = l.reports.iter().find(|r| r.0 == host);
            report.map(|(_, finished, scores)| (*finished, scores.clone()))
        });
        let complete = host_report.as_ref().is_none_or(|r| r.0);
        let key = self
            .playlists
            .iter()
            .find(|p| p.id == m.info.playlist)
            .map_or(String::new(), |p| p.key.clone());
        let seats: Vec<Seat> = m
            .seats
            .iter()
            .filter(|s| m.has(s.account))
            .copied()
            .collect();
        // Who left the game in progress (the host too, if it quit).
        let left = |a: u64| m.left(a) || m.host_quit && a == host;
        let host_result = m.results.iter().find(|(a, _)| *a == host);

        // The players the game is worth something to, as they finished,
        // and the XP each won or lost.
        let mut placed: Vec<(u64, Placed)> = Vec::new();
        let mut xp: Vec<(u64, i32)> = Vec::new();
        let mut counted = Counted::No;
        let reason = if let Some(why) = m.void {
            why
        } else if m.host_quit || m.lost_the_host() {
            self.matchmaker.host_left(host);
            if m.host_quit && m.started.is_some() && m.info.ranked {
                // The host comes last.
                placed = seats
                    .iter()
                    .map(|s| {
                        let p = Placed {
                            level: s.effective,
                            team: s.team,
                            place: u8::from(s.account == host),
                            bot: false,
                        };
                        (s.account, p)
                    })
                    .collect();
                let game: Vec<Placed> = placed.iter().map(|(_, p)| *p).collect();
                let changes = levels::xp_changes(&game, teams);
                let mine = placed.iter().zip(changes).find(|((a, _), _)| *a == host);
                xp.extend(mine.map(|((a, _), change)| (*a, change)));
                counted = Counted::HostLoss;
            }
            HOST_LEFT
        } else if let Some((_, result)) = host_result {
            // The host's result, for the players in the match, with those
            // who left last in free-for-all games.
            let mut finished: Vec<(&Seat, &PlayerResult)> = Vec::new();
            for r in result {
                let seat = seats.iter().find(|s| s.account == r.account);
                let twice = finished.iter().any(|(s, _)| s.account == r.account);
                if let (Some(seat), false) = (seat, twice) {
                    finished.push((seat, r));
                }
            }
            let finishes: Vec<Finish> = finished
                .iter()
                .map(|(_, r)| Finish {
                    team: 0,
                    score: r.score,
                    left: r.left || m.left(r.account),
                })
                .collect();
            let places = if teams {
                finished.iter().map(|(_, r)| r.place).collect()
            } else {
                levels::places(&finishes, false, None)
            };
            placed = finished
                .iter()
                .zip(places)
                .map(|((s, _), place)| {
                    let p = Placed {
                        level: s.effective,
                        team: s.team,
                        place,
                        bot: false,
                    };
                    (s.account, p)
                })
                .collect();
            let reports: Vec<&Vec<PlayerResult>> = m
                .results
                .iter()
                .filter(|(a, _)| *a != host)
                .map(|(_, r)| r)
                .collect();
            let agreeing = reports.iter().filter(|r| agrees(result, r)).count();
            // Unless every joined PC left the game, one must have
            // answered.
            let staying = m.joiners().filter(|&a| !m.left(a)).count();
            let confirmed = !reports.is_empty() || staying == 0;
            let game: Vec<Placed> = placed.iter().map(|(_, p)| *p).collect();
            let ranked = m.info.ranked;
            let counts = levels::counts(ranked, complete, agreeing, reports.len(), &game, teams);
            if counts && confirmed {
                counted = Counted::Yes;
                let changes = levels::xp_changes(&game, teams);
                xp = placed.iter().map(|(a, _)| *a).zip(changes).collect();
            }
            if !m.info.ranked {
                ""
            } else if !complete {
                UNFINISHED
            } else if agreeing * 2 < reports.len() {
                DISPUTED
            } else if !confirmed {
                UNCONFIRMED
            } else if !counts {
                NO_OPPONENTS
            } else {
                ""
            }
        } else {
            NO_RESULT
        };

        // Each player's rank before and after.
        let mut ranks: Vec<(u64, Rank, Rank)> = Vec::new();
        let mut changed = Vec::new();
        for (account, change) in xp {
            let Some(mut a) = self.accounts.get(&account).cloned() else {
                continue;
            };
            let place = placed
                .iter()
                .find(|(p, _)| *p == account)
                .map(|p| p.1.place);
            let won = place == Some(0) && placed.iter().any(|(_, p)| p.place > 0);
            let i = match a.stats.iter().position(|s| s.playlist == key) {
                Some(i) => i,
                None => {
                    a.stats.push(Stats {
                        playlist: key.clone(),
                        rank: Rank::default(),
                        games: 0,
                        wins: 0,
                    });
                    a.stats.len() - 1
                }
            };
            let stats = &mut a.stats[i];
            let old = stats.rank;
            stats.rank = old.after(change);
            stats.games += 1;
            stats.wins += u32::from(won);
            ranks.push((account, old, stats.rank));
            changed.push(a);
        }
        self.update_all(changed);

        let rank = |a: u64| {
            let stats = self.accounts.get(&a).and_then(|a| a.stats(&key));
            stats.map_or(Rank::default(), |s| s.rank)
        };
        let record = GameRecord {
            unix: self.epoch + now as u64,
            id,
            playlist: key.clone(),
            map: m.info.map.clone(),
            game_type: m.info.game_type,
            counted,
            players: seats
                .iter()
                .map(|s| {
                    let (old, new) = match ranks.iter().find(|r| r.0 == s.account) {
                        Some(&(_, old, new)) => (old, new),
                        None => (rank(s.account), rank(s.account)),
                    };
                    let place = placed.iter().find(|(a, _)| *a == s.account);
                    RecordedPlayer {
                        account: s.account,
                        team: s.team,
                        place: place.map_or(0, |p| p.1.place),
                        left: left(s.account),
                        old,
                        new,
                    }
                })
                .collect(),
        };
        let games = self.dir.join("games.log");
        if let Err(e) = store::log_game(&games, &record) {
            log(format_args!("live: can't log to {}: {e}", games.display()));
        }
        let bytes = self.relayed.remove(&id).unwrap_or(0);
        let relayed = match (&m.launcher, &self.relay) {
            (Some(launcher), relay) => {
                if relay.is_some() {
                    self.closing_rooms.push((launcher.room, now + ROOM_GRACE));
                }
                let scores = host_report.map_or(Vec::new(), |r| r.1);
                let scores: Vec<String> = scores.iter().map(i32::to_string).collect();
                let closing = format!("its relay room closes in {ROOM_GRACE:.0} s");
                match scores.len() {
                    0 => format!(", {closing}"),
                    _ => format!(", team scores {}, {closing}", scores.join("-")),
                }
            }
            (None, _) => format!(", {bytes} bytes relayed"),
        };
        log(format_args!(
            "live: match {id:016x} is over{}{}{relayed}",
            match counted {
                Counted::No => "",
                Counted::Yes => ", counted",
                Counted::HostLoss => ", a loss for its host",
            },
            if reason.is_empty() {
                String::new()
            } else {
                format!(": {reason}")
            }
        ));
        self.unlink(id, None);

        // Everyone hears everyone's new level, for their carnage report.
        let players: Vec<(u64, u8)> = ranks.iter().map(|&(a, _, new)| (a, new.level)).collect();
        for s in &seats {
            let mine = ranks.iter().find(|r| r.0 == s.account);
            let reason = match mine {
                Some(_) if s.account == host && m.host_quit => HOST_QUIT,
                None if counted == Counted::Yes => NOT_JOINED,
                _ => reason,
            };
            let over = MatchOver {
                id,
                counted: mine.is_some(),
                reason: reason.into(),
                card: self.card_of(s.account),
                levels: mine.map_or(Vec::new(), |&(_, old, new)| {
                    vec![(m.info.playlist, old.level, new.level)]
                }),
                players: players.clone(),
            };
            self.tell(s.account, &ToPc::MatchOver(over));
            if mine.is_some() {
                // Their levels everywhere else, as they are now.
                self.welcome(s.account);
                let playlists =
                    ToPc::Playlists(self.playlists_for(s.account, &self.playlist_counts()));
                self.tell(s.account, &playlists);
            }
        }
        for s in &m.seats {
            if let Some(k) = self.pc_of(s.account) {
                let party = self.pcs[k].party;
                if let Some(p) = self.parties.get_mut(&party) {
                    p.previous_map = Some(m.info.map.clone());
                }
                // Members' levels may have changed.
                self.party_changed(party);
            }
            self.free_party(s.account);
        }
    }

    /// `me`, leading their party, opens a custom game on a map everyone in
    /// it has: they host, and everyone else is linked to them. A member
    /// who left the party's custom game comes back into it.
    pub(super) fn custom(&mut self, me: u64) {
        if self.client_of(me) == ClientKind::Launcher {
            return self.notice(me, NO_CUSTOM_GAMES);
        }
        if let Some((id, party)) = self.party_of(me) {
            if party.activity == Activity::Custom && party.leader != me {
                let leader = party.leader;
                self.unlink(id, Some(me));
                return self.join_custom(id, leader, me);
            }
        }
        let Some((id, party)) = self.led_by(me) else {
            return;
        };
        if !matches!(party.activity, Activity::Lobby | Activity::Searching) {
            return;
        }
        let members = party.members.clone();
        if members.iter().any(|&a| self.busy(a)) {
            return self.notice(me, STILL_PLAYING);
        }
        let Some(party) = self.parties.get(&id) else {
            return;
        };
        let maps = self.shared_maps(party);
        let previous = party.previous_map.as_ref();
        let Some(map) = maps
            .iter()
            .find(|m| previous.is_some_and(|p| p.eq_ignore_ascii_case(m)))
            .or(maps.first())
            .cloned()
        else {
            return self.notice(me, NO_MAPS);
        };
        if party.activity == Activity::Searching {
            self.matchmaker.cancel(id);
        }
        if let Some(party) = self.parties.get_mut(&id) {
            party.activity = Activity::Custom;
            party.playlist = QUICKMATCH;
            party.custom_map = map.clone();
        }
        self.party_changed(id);
        self.relayed.insert(id, 0);
        log(format_args!(
            "live: party {id} opened a custom game on {map}"
        ));
        for account in members {
            self.join_custom(id, me, account);
        }
    }

    /// `me`, leading a party of launchers, starts a custom game of MCC
    /// variant `variant` on `map` for everyone in it (see the top).
    pub(super) fn launcher_custom(&mut self, me: u64, map: &str, variant: &str, now: f64) {
        let Some((id, party)) = self.led_by(me) else {
            return;
        };
        if !matches!(party.activity, Activity::Lobby | Activity::Searching) {
            return self.notice(me, STILL_PLAYING);
        }
        let searching = party.activity == Activity::Searching;
        // The leader first: they host.
        let mut members = vec![me];
        members.extend(party.members.iter().copied().filter(|&a| a != me));
        if members.iter().any(|&a| self.busy(a)) {
            return self.notice(me, STILL_PLAYING);
        }
        if members
            .iter()
            .any(|&a| self.client_of(a) != ClientKind::Launcher)
        {
            return self.notice(me, OTHER_PROGRAM);
        }
        if self.relay.is_none() {
            return self.notice(me, NO_RELAY);
        }
        let launchers = || {
            let playlists = self.playlists.iter();
            playlists.filter(|p| p.client == ClientKind::Launcher)
        };
        let named = |v: &&crate::playlists::Variant| {
            v.mcc
                .as_deref()
                .is_some_and(|m| m.eq_ignore_ascii_case(variant))
        };
        let Some(game) = launchers().find_map(|p| p.variants.iter().find(named).cloned()) else {
            return self.notice(me, NO_SUCH_GAME);
        };
        let played = launchers().any(|p| p.maps.iter().any(|m| m.eq_ignore_ascii_case(map)));
        let shared = self
            .parties
            .get(&id)
            .map_or(Vec::new(), |p| self.shared_maps(p));
        let map = shared
            .into_iter()
            .find(|m| played && m.eq_ignore_ascii_case(map));
        let hash = self.pc_of(me).and_then(|k| {
            let maps = self.pcs[k].maps.iter();
            maps.filter(|(m, _)| map.as_ref().is_some_and(|map| m.eq_ignore_ascii_case(map)))
                .map(|&(_, hash)| hash)
                .next()
        });
        let (Some(map), Some(hash)) = (map, hash) else {
            return self.notice(me, NO_SUCH_MAP);
        };
        if searching {
            self.matchmaker.cancel(id);
        }
        let teams = game.game_type.teams();
        let players = members.iter().enumerate().map(|(i, &account)| {
            let level = self.accounts.get(&account).map_or(1, |a| a.best_level());
            Seat {
                account,
                party: id,
                team: if teams { (i % 2) as u8 } else { 0 },
                level,
                effective: level,
                guests: 0,
            }
        });
        let m = matchmaker::Match {
            id: self.matchmaker.custom(me, now),
            playlist: CUSTOM_GAME,
            ranked: false,
            map,
            hash,
            variant: game,
            bots: 0,
            host: me,
            players: players.collect(),
        };
        log(format_args!("live: party {id} starts a custom game"));
        self.formed(m);
    }

    /// `account` comes into party `id`'s custom game, which `leader` hosts.
    pub(super) fn join_custom(&mut self, id: u64, leader: u64, account: u64) {
        let Some(party) = self.parties.get(&id) else {
            return;
        };
        let map = party.custom_map.clone();
        let open = ToPc::CustomOpen {
            party: id,
            leader,
            map: map.clone(),
        };
        self.tell(account, &open);
        if account != leader {
            let level = |a: u64| self.accounts.get(&a).map_or(1, |a| a.best_level());
            let host = (leader, level(leader), ANY_TEAM);
            self.link(id, &map, host, (account, level(account), ANY_TEAM));
        }
    }

    /// `me`, hosting their party's custom game, moves it to `map`.
    pub(super) fn custom_map(&mut self, me: u64, map: &str) {
        let Some((id, party)) = self.led_by(me) else {
            return;
        };
        if party.activity != Activity::Custom {
            return;
        }
        let Some(party) = self.parties.get(&id) else {
            return;
        };
        let shared = self.shared_maps(party);
        let Some(map) = shared.into_iter().find(|m| m.eq_ignore_ascii_case(map)) else {
            return self.notice(me, NO_MAPS);
        };
        if let Some(party) = self.parties.get_mut(&id) {
            party.custom_map = map.clone();
        }
        let open = ToPc::CustomOpen {
            party: id,
            leader: me,
            map,
        };
        // Those in it hear: members who left it hear again if they come
        // back.
        for account in self.members_of(id) {
            if account == me || self.has_link(id, account) {
                self.tell(account, &open);
            }
        }
    }

    /// `me` is back in the party lobby: if they host its custom game, it's
    /// over.
    pub(super) fn back(&mut self, me: u64) {
        if let Some((id, party)) = self.led_by(me) {
            if party.activity == Activity::Custom {
                self.close_custom(id);
            }
        }
    }

    /// Party `id`'s custom game is over: everyone goes back to the lobby.
    pub(super) fn close_custom(&mut self, id: u64) {
        let Some(party) = self.parties.get_mut(&id) else {
            return;
        };
        party.activity = Activity::Lobby;
        party.custom_map.clear();
        self.unlink(id, None);
        let bytes = self.relayed.remove(&id).unwrap_or(0);
        log(format_args!(
            "live: party {id} closed its custom game, {bytes} bytes relayed"
        ));
        self.party_changed(id);
    }

    /// People searching and playing each playlist.
    pub(super) fn playlist_counts(&self) -> Vec<(u16, u16)> {
        let count = |n: usize| n.min(u16::MAX as usize) as u16;
        self.playlists
            .iter()
            .map(|p| {
                let matches = self.matches.iter().filter(|m| m.info.playlist == p.id);
                let seats = matches.flat_map(|m| m.seats.iter().filter(|s| m.has(s.account)));
                let playing: usize = seats.map(|s| 1 + usize::from(s.guests)).sum();
                (count(self.matchmaker.searching(p.id)), count(playing))
            })
            .collect()
    }

    /// Tell everyone the playlists again if the counts in them changed, at
    /// most every few seconds.
    pub(super) fn send_playlists(&mut self, now: f64) {
        if now - self.playlists_sent < PLAYLISTS_EVERY {
            return;
        }
        let counts = self.playlist_counts();
        if counts == self.counts_sent {
            return;
        }
        let signed_in: Vec<u64> = self.pcs.iter().filter_map(|pc| pc.account).collect();
        for account in signed_in {
            let playlists = ToPc::Playlists(self.playlists_for(account, &counts));
            self.tell(account, &playlists);
        }
        self.counts_sent = counts;
        self.playlists_sent = now;
    }
}
