//! Matches, from the search to the levels won and lost. A party's leader
//! searches a playlist; the matchmaker puts parties together, and everyone
//! in the match hears of it (MATCH) while the PC with the best connection
//! is asked to host it (HOST_MATCH). Once that PC says it is (HOSTING),
//! every other PC is linked to it through the relay (LINK), and the game
//! starts (GO) when all have come, or 20 seconds on: a PC that hasn't come
//! by then is left out of it. When the game ends each PC says how it saw it
//! end (RESULT). If at least half of the joined PCs that answer within 30
//! seconds saw it as the host did, levels change as `levels` says, and
//! everyone hears how (MATCH_OVER).
//!
//! A host that leaves ends the match: it doesn't count for anyone, except
//! as a loss for the host if it quit or its link to the server died. A
//! player who quits a game in progress loses it as `levels` says: last in
//! free-for-all, with their team otherwise.
//!
//! Parties play custom games too: the leader hosts on a map everyone has,
//! and every member is linked to them, those who join later as well. They
//! change no levels.

use super::Server;
use crate::card;
use crate::levels::{self, Finish, Placed, Rank, MAX_LEVEL};
use crate::matchmaker::{self, Event, Member, Seat, Ticket};
use crate::store::{self, GameRecord, RecordedPlayer, Stats};
use h2net::live::{
    Activity, MatchInfo, MatchOver, MatchPlayer, PlayerResult, SearchStatus, Stage, ToPc,
    QUICKMATCH,
};
use h2net::ANY_TEAM;

/// Seconds from HOSTING to GO at the latest, for PCs slow to link.
const LINK_WAIT: f64 = 20.0;
/// Seconds after the first PC reports (a result, or the host lost) that
/// the others have to send their results.
const RESULTS_WAIT: f64 = 30.0;
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
const NO_OPPONENTS: &str = "NO ONE PLAYED AGAINST YOU. THE GAME DIDN'T COUNT.";
const NO_HOST: &str = "NO ONE COULD HOST THE GAME.";
const NOT_JOINED: &str = "YOU DIDN'T JOIN THE GAME.";

/// A match from the moment it formed until everyone hears how it went.
pub(super) struct Match {
    /// As the players were told (MATCH).
    pub(super) info: MatchInfo,
    /// Every PC in it, as the matchmaker seated them.
    pub(super) seats: Vec<Seat>,
    /// When the host said it's hosting.
    hosting: Option<f64>,
    /// The game started (GO).
    started: bool,
    /// Joining PCs whose link to the host came.
    linked: Vec<u64>,
    /// PCs out of it before the game started, who were told it didn't
    /// count for them.
    pub(super) out: Vec<u64>,
    /// PCs that quit the game in progress, and joining PCs that lost the
    /// host.
    quit: Vec<u64>,
    lost_host: Vec<u64>,
    /// The host quit, through its menu or because its link to the server
    /// died.
    host_quit: bool,
    /// Why it can't count for anyone, if it can't.
    void: Option<&'static str>,
    /// Each PC's RESULT, and when the first PC reported.
    results: Vec<(u64, Vec<PlayerResult>)>,
    first_report: Option<f64>,
}

impl Match {
    fn new(info: MatchInfo, seats: Vec<Seat>) -> Match {
        Match {
            info,
            seats,
            hosting: None,
            started: false,
            linked: Vec::new(),
            out: Vec::new(),
            quit: Vec::new(),
            lost_host: Vec::new(),
            host_quit: false,
            void: None,
            results: Vec::new(),
            first_report: None,
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

    /// Everyone in the game has had their say: a result, or they're gone.
    fn all_in(&self) -> bool {
        let gone = |a: u64| self.quit.contains(&a) || self.lost_host.contains(&a);
        (self.host_quit || self.reported(self.info.host))
            && self.joiners().all(|a| self.reported(a) || gone(a))
    }

    /// At least half the joining PCs lost the host: it's gone.
    fn lost_the_host(&self) -> bool {
        let joiners = self.joiners().count();
        self.started && joiners > 0 && self.lost_host.len() * 2 >= joiners
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
        let gone = |m: &Match| m.quit.contains(&account) || m.lost_host.contains(&account);
        self.matches.iter().any(|m| m.has(account) && !gone(m))
    }

    /// `me`, leading their party, starts it searching `playlist` (or
    /// `QUICKMATCH`).
    pub(super) fn search(&mut self, me: u64, playlist: u8, now: f64) {
        let Some((id, party)) = self.led_by(me) else {
            return;
        };
        if !matches!(party.activity, Activity::Lobby | Activity::Searching) {
            return;
        }
        let (members, previous_map) = (party.members.clone(), party.previous_map.clone());
        if members.iter().any(|&a| self.busy(a)) {
            return self.notice(me, STILL_PLAYING);
        }
        let members = members.iter().filter_map(|&a| {
            let pc = &self.pcs[self.pc_of(a)?];
            let account = self.accounts.get(&a)?;
            let ranked = self.playlists.iter().filter(|p| p.ranked);
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
                // Everyone hears who hosts now.
                self.matches[k].info.host = host;
                let info = self.matches[k].info.clone();
                for account in info.players.iter().map(|p| p.account) {
                    if self.matches[k].has(account) {
                        self.tell(account, &ToPc::Match(info.clone()));
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

    /// The matchmaker made match `m`: tell everyone in it, and ask its host
    /// to host.
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
        println!(
            "live: match {:016x}: {:?} on {}, {} PCs and {} bots",
            m.id,
            m.variant.game_type,
            m.map,
            m.players.len(),
            m.bots
        );
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
        for s in &m.players {
            self.tell(s.account, &ToPc::Match(info.clone()));
            self.tell(s.account, &ToPc::Status(joining));
        }
        self.tell(m.host, &ToPc::HostMatch(m.id));
        self.relayed.insert(m.id, 0);
        self.matches.push(Match::new(info, m.players));
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
        self.tell(me, &ToPc::Status(starting));
        for j in joiners {
            self.link(
                id,
                &map,
                (me, host.level, host.team),
                (j.account, j.level, j.team),
            );
            self.tell(j.account, &ToPc::Status(starting));
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

    /// `me` says how match `id` ended (the first time only).
    pub(super) fn result(&mut self, me: u64, id: u64, players: Vec<PlayerResult>) {
        if let Some(m) = self.matches.iter_mut().find(|m| m.info.id == id) {
            if m.started && m.has(me) && !m.reported(me) {
                m.results.push((me, players));
            }
        }
    }

    /// `me` left match `id` before its end: it quit, or lost the host.
    pub(super) fn left_match(&mut self, me: u64, id: u64, host_lost: bool) {
        if let Some(k) = self.matches.iter().position(|m| m.info.id == id) {
            if self.matches[k].has(me) {
                self.leave(k, me, host_lost);
            }
        }
    }

    /// `account`'s link to the server died: it quit whatever match it was
    /// in.
    pub(super) fn gone(&mut self, account: u64) {
        for k in 0..self.matches.len() {
            let m = &self.matches[k];
            let gone = m.quit.contains(&account) || m.lost_host.contains(&account);
            if m.has(account) && !gone {
                self.leave(k, account, false);
            }
        }
    }

    /// `account` left match `k`, having lost the host if `host_lost`.
    fn leave(&mut self, k: usize, account: u64, host_lost: bool) {
        let m = &mut self.matches[k];
        if account == m.info.host && m.hosting.is_some() {
            m.host_quit = true;
        } else if !m.started {
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
        };
        self.tell(account, &ToPc::MatchOver(over));
        self.free_party(account);
    }

    fn card_of(&self, account: u64) -> String {
        self.accounts
            .get(&account)
            .map_or(String::new(), |a| card::sign(a, &self.key))
    }

    /// `account`'s party goes back to its lobby, if it was playing a match
    /// and none of its members is any more.
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
                party.playlist = QUICKMATCH;
            }
            self.party_changed(id);
        }
    }

    /// Move matches along: ask another PC to host if the one asked left,
    /// start games, and end matches whose results are in (or can't come).
    pub(super) fn run_matches(&mut self, now: f64) {
        let mut k = 0;
        while k < self.matches.len() {
            let m = &mut self.matches[k];
            let id = m.info.id;
            let reported = !m.results.is_empty() || !m.lost_host.is_empty();
            if reported && m.first_report.is_none() {
                m.first_report = Some(now);
            }
            if m.hosting.is_none() && m.void.is_none() && m.out.contains(&m.info.host) {
                let next = self.matchmaker.host_failed(id, now);
                self.matchmaker_event(next.unwrap_or(Event::NoHost { id }), now);
                continue;
            }
            let m = &self.matches[k];
            let all_linked = m.joiners().all(|a| m.linked.contains(&a));
            if !m.started
                && m.hosting
                    .is_some_and(|t| all_linked || now >= t + LINK_WAIT)
            {
                self.go(k);
            }
            let m = &self.matches[k];
            let over = m.void.is_some()
                || m.host_quit
                || m.lost_the_host()
                || m.started && m.all_in()
                || m.first_report.is_some_and(|t| now >= t + RESULTS_WAIT);
            if over {
                self.end(k, now);
            } else {
                k += 1;
            }
        }
    }

    /// Start match `k`'s game, without PCs that aren't linked to the host.
    fn go(&mut self, k: usize) {
        let m = &self.matches[k];
        let late: Vec<u64> = m.joiners().filter(|a| !m.linked.contains(a)).collect();
        for account in late {
            self.leave_early(k, account);
        }
        let m = &mut self.matches[k];
        m.started = true;
        let (id, players) = (m.info.id, m.seats.clone());
        for s in players {
            if self.matches[k].has(s.account) {
                self.tell(s.account, &ToPc::Go(id));
            }
        }
    }

    /// Match `k` is over: work out what it was worth to each player, keep
    /// it, and tell them.
    fn end(&mut self, k: usize, now: f64) {
        let m = self.matches.remove(k);
        let (id, host, teams) = (m.info.id, m.info.host, m.info.game_type.teams());
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
        let left = |a: u64| m.quit.contains(&a) || m.lost_host.contains(&a);
        let host_result = m.results.iter().find(|(a, _)| *a == host);

        // The players the game is worth something to, as they finished,
        // and the XP each won or lost.
        let mut placed: Vec<(u64, Placed)> = Vec::new();
        let mut xp: Vec<(u64, i32)> = Vec::new();
        let mut counted = false;
        let reason = if let Some(why) = m.void {
            why
        } else if m.host_quit || m.lost_the_host() {
            self.matchmaker.host_left(host);
            if m.host_quit && m.started && m.info.ranked {
                // The host comes last.
                let game: Vec<Placed> = seats
                    .iter()
                    .map(|s| Placed {
                        level: s.effective,
                        team: s.team,
                        place: u8::from(s.account == host),
                        bot: false,
                    })
                    .collect();
                let changes = levels::xp_changes(&game, teams);
                let mine = seats.iter().zip(changes).find(|(s, _)| s.account == host);
                xp.extend(mine.map(|(s, change)| (s.account, change)));
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
                    left: r.left || left(r.account),
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
            let game: Vec<Placed> = placed.iter().map(|(_, p)| *p).collect();
            counted = levels::counts(m.info.ranked, true, agreeing, reports.len(), &game, teams);
            if counted {
                let changes = levels::xp_changes(&game, teams);
                xp = placed.iter().map(|(a, _)| *a).zip(changes).collect();
            }
            if !m.info.ranked {
                ""
            } else if agreeing * 2 < reports.len() {
                DISPUTED
            } else if !counted {
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
        let log = self.dir.join("games.log");
        if let Err(e) = store::log_game(&log, &record) {
            println!("live: can't log to {}: {e}", log.display());
        }
        let bytes = self.relayed.remove(&id).unwrap_or(0);
        println!(
            "live: match {id:016x} is over{}{}, {bytes} bytes relayed",
            if counted { ", counted" } else { "" },
            if reason.is_empty() {
                String::new()
            } else {
                format!(": {reason}")
            }
        );
        self.unlink(id, None);

        for s in &seats {
            let mine = ranks.iter().find(|r| r.0 == s.account);
            let reason = match mine {
                Some(_) if s.account == host && m.host_quit => HOST_QUIT,
                None if counted => NOT_JOINED,
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
    /// it has: they host, and everyone else is linked to them.
    pub(super) fn custom(&mut self, me: u64) {
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
        println!("live: party {id} opened a custom game on {map}");
        for account in members {
            self.join_custom(id, me, account);
        }
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
        self.tell_party(id, &open);
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
        println!("live: party {id} closed its custom game, {bytes} bytes relayed");
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
