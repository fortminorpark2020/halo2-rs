//! Matchmaking, the way Halo 2 did it: parties search a playlist, and four
//! times a second the oldest searching party gathers every other party that
//! fits into a match. A party fits if the match has room for it, everyone
//! has a map from the playlist in common (the same file, by its hash), and
//! in ranked playlists their levels are close enough; that range widens the
//! longer the match gathers. A full match starts at once, and one with its
//! fewest players counts down first, a little longer for each party that
//! arrives. Then the parties are split into teams, a map and game are
//! picked, and the PC with the best connection is asked to host.
//!
//! Nothing here reads a clock: the caller passes the time in seconds, so
//! tests can run minutes of searching in an instant.

use crate::levels::{self, MAX_LEVEL};
use crate::playlists::{Bots, Playlist, Variant};
use std::cmp::Reverse;
use std::collections::HashMap;

/// The playlist number that asks for Quickmatch: whichever playlist the
/// party can search has the most people searching.
pub const QUICKMATCH: u8 = 255;

/// Seconds between searches.
const TICK: f64 = 0.25;
/// Ranked level ranges widen by `WIDEN_LEVELS` every `WIDEN_EVERY` seconds a
/// match has gathered, and levels stop mattering after `ANY_LEVEL_AFTER`.
const WIDEN_EVERY: f64 = 15.0;
const WIDEN_LEVELS: u8 = 3;
const ANY_LEVEL_AFTER: f64 = 60.0;
/// Seconds from a match having its fewest players to its start, more for
/// each party that arrives meanwhile, but never more than
/// `LONGEST_COUNTDOWN` in all.
const COUNTDOWN: f64 = 20.0;
const ARRIVAL_TIME: f64 = 5.0;
const LONGEST_COUNTDOWN: f64 = 40.0;
/// Seconds a party searches before it gives up.
const GIVE_UP: f64 = 600.0;
/// Seconds the PC asked to host has to start hosting before the next is
/// asked.
const HOSTING_TIMEOUT: f64 = 45.0;
/// PCs within this many milliseconds of the fastest are as good a host,
/// and the one in the biggest party is asked first.
const RTT_TIE: u32 = 10;
/// Matches a PC is asked to host last after its hosted match ended because
/// it left.
const HOST_PENALTY: u8 = 3;
/// Most people and bots in a game.
const MAX_PLAYERS: usize = 16;

/// Why a party can't search, mostly in Halo 2's words.
const INVALID: &str = "Invalid matchmaking playlist";
const TOO_LARGE: &str = "Your party is too large to enter this matchmaking playlist";
const NO_RANKED_GUESTS: &str = "Guests are not allowed in ranked matchmade games";
const NO_GUESTS: &str = "Guests are not allowed in this matchmaking playlist";
const NO_MAPS: &str = "Your party has no map from this playlist in common";
const NO_PLAYLIST: &str = "No matchmaking playlist fits your party";

/// A party asking for a match.
#[derive(Debug, Clone)]
pub struct Ticket {
    pub party: u64,
    /// The playlist's id, or `QUICKMATCH`.
    pub playlist: u8,
    /// The party's PCs, one for each signed-in player.
    pub members: Vec<Member>,
    /// The map the party played last, which its next match won't be on.
    pub previous_map: Option<String>,
}

/// A player in a searching party, and their PC.
#[derive(Debug, Clone)]
pub struct Member {
    pub account: u64,
    /// Their level in each ranked playlist they've played, by playlist id.
    /// Elsewhere they're level 1.
    pub levels: Vec<(u8, u8)>,
    /// Splitscreen guests playing on their PC.
    pub guests: u8,
    /// The maps their PC has: file name and a hash of the file.
    pub maps: Vec<(String, u64)>,
    /// The middle of their PC's recent round trips to the server, in
    /// milliseconds.
    pub rtt: u32,
}

impl Member {
    /// Their level in `playlist`. Unranked playlists have none, so there
    /// it's their best, which still helps to even out teams.
    fn level(&self, playlist: &Playlist) -> u8 {
        let level = if playlist.ranked {
            let own = self.levels.iter().find(|&&(id, _)| id == playlist.id);
            own.map(|&(_, level)| level)
        } else {
            self.levels.iter().map(|&(_, level)| level).max()
        };
        level.unwrap_or(1).clamp(1, MAX_LEVEL)
    }

    /// People playing on their PC.
    fn people(&self) -> usize {
        1 + usize::from(self.guests)
    }
}

/// Where a search is, numbered as STATUS messages send it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// No one else yet: "Searching for the best possible game...".
    Searching = 0,
    /// With others, but short of the playlist's fewest: "Waiting for
    /// additional players...".
    Gathering = 1,
    /// "Waiting for game to fill up, start in %02d:%02d".
    WaitingToFill = 2,
    /// Enough people, but the teams can't be even: "%d more player(s)
    /// needed to balance teams".
    Balancing = 3,
}

/// What a searching party is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    pub stage: Stage,
    /// People in the match being gathered.
    pub have: u8,
    /// More people it needs before it can start.
    pub need: u8,
    /// Seconds until the start while waiting to fill; otherwise seconds
    /// searched so far.
    pub seconds: u16,
    /// The lowest and highest level a player could have to join.
    pub low: u8,
    pub high: u8,
}

/// What the server should tell the players.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// What a searching party should show now (sent when it changes).
    Status { party: u64, status: Status },
    /// A match is ready: tell everyone in it, and ask its host to host.
    Formed(Match),
    /// A party searched for ten minutes and stopped: "Matchmaking failed!".
    Failed { party: u64 },
    /// Match `id`'s host didn't start hosting: ask `host` instead.
    NewHost { id: u64, host: u64 },
    /// No one in match `id` could host it.
    NoHost { id: u64 },
}

/// A match ready to play.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub id: u64,
    /// The playlist's id.
    pub playlist: u8,
    pub ranked: bool,
    /// The map's file name, and the hash everyone's copy has.
    pub map: String,
    pub hash: u64,
    pub variant: Variant,
    pub bots: u8,
    /// The PC asked to host first.
    pub host: u64,
    /// Every PC in the match, party by party.
    pub players: Vec<Seat>,
}

/// A PC in a match, with its guests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seat {
    pub account: u64,
    pub party: u64,
    /// 0 or 1 in team games; 0 for everyone in free-for-all.
    pub team: u8,
    /// Their level in the playlist, and the level they count as for XP
    /// (higher in a party with a much higher level, see
    /// `levels::effective_level`).
    pub level: u8,
    pub effective: u8,
    pub guests: u8,
}

/// A small random number generator (xorshift64*), seeded by the caller so
/// tests can replay the same choices.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        // Zero would stay zero.
        Rng(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number below `n` (which isn't 0).
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// A party searching.
struct Search {
    ticket: Ticket,
    /// The playlist (an index into `Matchmaker::playlists`).
    playlist: usize,
    since: f64,
    people: usize,
    /// Each member's level in the playlist, and the level they count as.
    levels: Vec<(u8, u8)>,
    /// The playlist's maps (by index) that every member has, with the hash
    /// all their copies share.
    maps: Vec<(usize, u64)>,
    /// What the party was last shown.
    shown: Option<Status>,
}

/// Parties gathered into a match that hasn't started yet.
struct Group {
    playlist: usize,
    /// Indexes into `Matchmaker::searches`, the oldest party first.
    parties: Vec<usize>,
    /// People in each party.
    sizes: Vec<usize>,
    people: usize,
    /// The levels everyone counts as.
    levels: Vec<u8>,
    /// Maps everyone has.
    maps: Vec<(usize, u64)>,
    /// Levels the range has widened by, or `None` once levels don't matter.
    widened: Option<u8>,
}

impl Group {
    fn add(&mut self, i: usize, search: &Search) {
        self.parties.push(i);
        self.sizes.push(search.people);
        self.people += search.people;
        self.levels
            .extend(search.levels.iter().map(|&(_, level)| level));
        self.maps.retain(|m| search.maps.contains(m));
    }
}

/// A match with enough players counting down to its start.
struct Countdown {
    /// When it began and when it ends.
    began: f64,
    ends: f64,
    /// The parties in the match when last checked, to see who arrives.
    parties: Vec<u64>,
}

/// A formed match waiting for its host to start hosting.
struct Hosting {
    id: u64,
    /// The PCs to ask, best first, and which is asked now.
    hosts: Vec<u64>,
    asked: usize,
    deadline: f64,
}

/// The parties searching, and the matches waiting for a host.
pub struct Matchmaker {
    playlists: Vec<Playlist>,
    searches: Vec<Search>,
    countdowns: Vec<Countdown>,
    hosting: Vec<Hosting>,
    /// PCs asked to host last, and for how many more matches.
    host_penalties: HashMap<u64, u8>,
    rng: Rng,
    next_tick: f64,
}

/// Levels a range has widened by after gathering for `seconds`, or `None`
/// once levels don't matter.
fn widening(seconds: f64) -> Option<u8> {
    if seconds >= ANY_LEVEL_AFTER {
        None
    } else {
        Some(WIDEN_LEVELS * (seconds / WIDEN_EVERY) as u8)
    }
}

/// The playlist's maps (by index) that all of `members` have, with the hash
/// all their copies share.
fn shared_maps(members: &[Member], playlist: &Playlist) -> Vec<(usize, u64)> {
    let has = |m: &Member, name: &str, hash: u64| {
        m.maps
            .iter()
            .any(|(n, h)| *h == hash && n.eq_ignore_ascii_case(name))
    };
    let Some(first) = members.first() else {
        return Vec::new();
    };
    let mut shared = Vec::new();
    for (i, name) in playlist.maps.iter().enumerate() {
        let mut hashes = first
            .maps
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(name));
        if let Some(&(_, hash)) = hashes.find(|(_, h)| members.iter().all(|m| has(m, name, *h))) {
            shared.push((i, hash));
        }
    }
    shared
}

/// How many more people (each searching alone) parties of `sizes` need
/// before they split into teams the playlist allows, or `None` if they
/// never can. Ranked team games need teams the same size, or one apart when
/// a bot evens them out; anything else is always fine.
fn balance_needed(playlist: &Playlist, sizes: &[usize]) -> Option<usize> {
    if !(playlist.ranked && playlist.teams) {
        return Some(0);
    }
    let apart = usize::from(playlist.bots == Bots::Even);
    let total: usize = sizes.iter().sum();
    // Bit n is set when some of the parties add up to n people.
    let mut sums: u64 = 1;
    for &size in sizes {
        sums |= sums << size;
    }
    let room = usize::from(playlist.max).saturating_sub(total);
    (0..=room).find(|&extra| {
        (0..=total).filter(|&red| sums >> red & 1 == 1).any(|red| {
            (0..=extra).any(|more| (red + more).abs_diff(total - red + extra - more) <= apart)
        })
    })
}

/// The lowest and highest levels that can play everyone at the levels in
/// `players`, with ranges widened by `widened` (any level once that's
/// `None`).
fn level_span(players: &[u8], widened: Option<u8>) -> (u8, u8) {
    let Some(widened) = widened else {
        return (1, MAX_LEVEL);
    };
    let fits = |l: u8| players.iter().all(|&m| levels::can_match(l, m, widened));
    let low = (1..=MAX_LEVEL).find(|&l| fits(l)).unwrap_or(1);
    let high = (1..=MAX_LEVEL)
        .rev()
        .find(|&l| fits(l))
        .unwrap_or(MAX_LEVEL);
    (low, high)
}

/// Which team (0 or 1) each party plays on. Parties stay whole. Of the
/// splits whose teams are closest in size, the one whose summed levels are
/// closest wins, at random among equals.
fn split(sizes: &[usize], levels: &[u32], rng: &mut Rng) -> Vec<u8> {
    let mut best = Vec::new();
    let mut closest = (usize::MAX, u32::MAX);
    for split in 0..1u32 << sizes.len() {
        let (mut people, mut level) = ([0; 2], [0; 2]);
        for (i, (&size, &sum)) in sizes.iter().zip(levels).enumerate() {
            let team = (split >> i & 1) as usize;
            people[team] += size;
            level[team] += sum;
        }
        let apart = (people[0].abs_diff(people[1]), level[0].abs_diff(level[1]));
        if apart < closest {
            closest = apart;
            best.clear();
        }
        if apart == closest {
            best.push(split);
        }
    }
    let split = best[rng.below(best.len())];
    (0..sizes.len()).map(|i| (split >> i & 1) as u8).collect()
}

/// The order to ask PCs (account, round trip, people in its party) to host:
/// fastest first, but one within `RTT_TIE` of the fastest goes first if its
/// party is bigger. PCs in `penalties` go after all the others.
fn host_order(pcs: &[(u64, u32, usize)], penalties: &HashMap<u64, u8>) -> Vec<u64> {
    let (mut first, mut last): (Vec<(u64, u32, usize)>, Vec<_>) = pcs
        .iter()
        .partition(|(account, _, _)| !penalties.contains_key(account));
    let mut order = Vec::new();
    for pcs in [&mut first, &mut last] {
        while let Some(fastest) = pcs.iter().map(|pc| pc.1).min() {
            let best = (0..pcs.len())
                .filter(|&k| pcs[k].1 <= fastest.saturating_add(RTT_TIE))
                .max_by_key(|&k| (pcs[k].2, Reverse(pcs[k].1), Reverse(pcs[k].0)));
            let Some(best) = best else { break };
            order.push(pcs.remove(best).0);
        }
    }
    order
}

impl Matchmaker {
    /// A matchmaker for `playlists`, making its random choices from `seed`.
    pub fn new(playlists: Vec<Playlist>, seed: u64) -> Matchmaker {
        Matchmaker {
            playlists,
            searches: Vec::new(),
            countdowns: Vec::new(),
            hosting: Vec::new(),
            host_penalties: HashMap::new(),
            rng: Rng::new(seed),
            next_tick: 0.0,
        }
    }

    pub fn playlists(&self) -> &[Playlist] {
        &self.playlists
    }

    /// People searching the playlist with this id.
    pub fn searching(&self, playlist: u8) -> usize {
        match self.playlists.iter().position(|p| p.id == playlist) {
            Some(i) => self.searching_in(i),
            None => 0,
        }
    }

    fn searching_in(&self, playlist: usize) -> usize {
        let searches = self.searches.iter().filter(|s| s.playlist == playlist);
        searches.map(|s| s.people).sum()
    }

    /// Start `ticket`'s party searching at time `now`, instead of any search
    /// it had. Returns the id of the playlist it searches (Quickmatch's
    /// pick), or why it can't search.
    pub fn search(&mut self, ticket: Ticket, now: f64) -> Result<u8, &'static str> {
        self.cancel(ticket.party);
        let i = if ticket.playlist == QUICKMATCH {
            self.quickmatch(&ticket).ok_or(NO_PLAYLIST)?
        } else {
            let i = self.playlists.iter().position(|p| p.id == ticket.playlist);
            let i = i.ok_or(INVALID)?;
            if let Some(refusal) = self.refusal(&ticket, i) {
                return Err(refusal);
            }
            i
        };
        let playlist = &self.playlists[i];
        let own: Vec<u8> = ticket.members.iter().map(|m| m.level(playlist)).collect();
        let highest = own.iter().copied().max().unwrap_or(1);
        self.searches.push(Search {
            people: ticket.members.iter().map(Member::people).sum(),
            levels: own
                .iter()
                .map(|&level| (level, levels::effective_level(level, highest)))
                .collect(),
            maps: shared_maps(&ticket.members, playlist),
            ticket,
            playlist: i,
            since: now,
            shown: None,
        });
        Ok(playlist.id)
    }

    /// Why `ticket`'s party can't search playlist `i`, if it can't.
    fn refusal(&self, ticket: &Ticket, i: usize) -> Option<&'static str> {
        let playlist = &self.playlists[i];
        let people: usize = ticket.members.iter().map(Member::people).sum();
        let guests = ticket.members.iter().any(|m| m.guests > 0);
        if ticket.members.is_empty() {
            Some(INVALID)
        } else if people > usize::from(playlist.party_max.min(playlist.max))
            || balance_needed(playlist, &[people]).is_none()
        {
            Some(TOO_LARGE)
        } else if guests && playlist.ranked {
            Some(NO_RANKED_GUESTS)
        } else if guests && !playlist.guests {
            Some(NO_GUESTS)
        } else if shared_maps(&ticket.members, playlist).is_empty() {
            Some(NO_MAPS)
        } else {
            None
        }
    }

    /// The playlist Quickmatch picks for `ticket`: of those the party can
    /// search, the one with the most people searching, at random among the
    /// busiest.
    fn quickmatch(&mut self, ticket: &Ticket) -> Option<usize> {
        let open: Vec<usize> = (0..self.playlists.len())
            .filter(|&i| self.refusal(ticket, i).is_none())
            .collect();
        let busiest = open.iter().map(|&i| self.searching_in(i)).max()?;
        let open: Vec<usize> = open
            .into_iter()
            .filter(|&i| self.searching_in(i) == busiest)
            .collect();
        Some(open[self.rng.below(open.len())])
    }

    /// Stop `party` searching. Returns whether it was.
    pub fn cancel(&mut self, party: u64) -> bool {
        let before = self.searches.len();
        self.searches.retain(|s| s.ticket.party != party);
        self.searches.len() != before
    }

    /// `host` started hosting match `id`: its HOSTING arrived. Returns
    /// whether it's the PC that was asked; then the matchmaker is done with
    /// the match.
    pub fn hosting(&mut self, id: u64, host: u64) -> bool {
        let asked = |h: &Hosting| h.id == id && h.hosts[h.asked] == host;
        let Some(k) = self.hosting.iter().position(asked) else {
            return false;
        };
        self.hosting.remove(k);
        true
    }

    /// The PC asked to host match `id` can't (it refused, left or took too
    /// long). Returns who to ask next, or that no one is left (and the match
    /// is forgotten), or `None` for a match that isn't waiting for a host.
    pub fn host_failed(&mut self, id: u64, now: f64) -> Option<Event> {
        let k = self.hosting.iter().position(|h| h.id == id)?;
        let hosting = &mut self.hosting[k];
        hosting.asked += 1;
        if let Some(&host) = hosting.hosts.get(hosting.asked) {
            hosting.deadline = now + HOSTING_TIMEOUT;
            Some(Event::NewHost { id, host })
        } else {
            self.hosting.remove(k);
            Some(Event::NoHost { id })
        }
    }

    /// A match `account` hosted ended because it left: it's asked to host
    /// last in its next few matches.
    pub fn host_left(&mut self, account: u64) {
        self.host_penalties.insert(account, HOST_PENALTY);
    }

    /// Search, if a quarter of a second has passed since the last time, and
    /// say what happened.
    pub fn poll(&mut self, now: f64) -> Vec<Event> {
        let mut events = Vec::new();
        if now < self.next_tick {
            return events;
        }
        self.next_tick = now + TICK;
        let late: Vec<u64> = self
            .hosting
            .iter()
            .filter(|h| now >= h.deadline)
            .map(|h| h.id)
            .collect();
        for id in late {
            events.extend(self.host_failed(id, now));
        }

        let mut countdowns = Vec::new();
        // Searches that ended in a match or gave up.
        let mut ended = Vec::new();
        for group in self.gather(now) {
            let playlist = &self.playlists[group.playlist];
            let max = usize::from(playlist.max);
            let short = usize::from(playlist.min).saturating_sub(group.people);
            let need = short.max(balance_needed(playlist, &group.sizes).unwrap_or(0));
            let mut countdown = None;
            if need == 0 {
                let parties: Vec<u64> = group
                    .parties
                    .iter()
                    .map(|&i| self.searches[i].ticket.party)
                    .collect();
                // A countdown carries on while any party in it is still
                // here, even if the oldest left or an older one took over.
                let old = self
                    .countdowns
                    .iter()
                    .position(|c| c.parties.iter().any(|p| parties.contains(p)));
                let mut c = match old {
                    Some(k) => self.countdowns.remove(k),
                    None => Countdown {
                        began: now,
                        ends: now + COUNTDOWN,
                        parties: parties.clone(),
                    },
                };
                let arrived = parties.iter().filter(|p| !c.parties.contains(p)).count();
                c.ends = (c.ends + arrived as f64 * ARRIVAL_TIME).min(c.began + LONGEST_COUNTDOWN);
                c.parties = parties;
                if group.people == max || now >= c.ends {
                    let formed = self.form(&group, now);
                    events.push(Event::Formed(formed));
                    ended.extend(&group.parties);
                    continue;
                }
                countdown = Some(c);
            }
            for &i in &group.parties {
                let search = &self.searches[i];
                let party = search.ticket.party;
                if countdown.is_none() && now - search.since >= GIVE_UP {
                    events.push(Event::Failed { party });
                    ended.push(i);
                    continue;
                }
                let status = self.status(&group, search, need, countdown.as_ref(), now);
                if search.shown != Some(status) {
                    self.searches[i].shown = Some(status);
                    events.push(Event::Status { party, status });
                }
            }
            countdowns.extend(countdown);
        }
        self.countdowns = countdowns;
        ended.sort_unstable();
        for i in ended.into_iter().rev() {
            self.searches.remove(i);
        }
        events
    }

    /// Gather the searching parties into matches: the oldest party not yet
    /// in one starts the next, and takes in every younger party that fits,
    /// oldest first.
    fn gather(&self, now: f64) -> Vec<Group> {
        let mut order: Vec<usize> = (0..self.searches.len()).collect();
        order.sort_by(|&a, &b| self.searches[a].since.total_cmp(&self.searches[b].since));
        let mut taken = vec![false; order.len()];
        let mut groups = Vec::new();
        for (k, &first) in order.iter().enumerate() {
            if taken[first] {
                continue;
            }
            taken[first] = true;
            let search = &self.searches[first];
            let mut group = Group {
                playlist: search.playlist,
                parties: Vec::new(),
                sizes: Vec::new(),
                people: 0,
                levels: Vec::new(),
                maps: search.maps.clone(),
                widened: widening(now - search.since),
            };
            group.add(first, search);
            for &other in &order[k + 1..] {
                if !taken[other] && self.fits(&group, &self.searches[other]) {
                    taken[other] = true;
                    group.add(other, &self.searches[other]);
                }
            }
            groups.push(group);
        }
        groups
    }

    /// Whether a searching party can join a group.
    fn fits(&self, group: &Group, search: &Search) -> bool {
        let playlist = &self.playlists[group.playlist];
        if search.playlist != group.playlist
            || group.people + search.people > usize::from(playlist.max)
            || !group.maps.iter().any(|m| search.maps.contains(m))
        {
            return false;
        }
        let levels_fit = match group.widened {
            Some(widened) if playlist.ranked => group.levels.iter().all(|&a| {
                let mut theirs = search.levels.iter();
                theirs.all(|&(_, b)| levels::can_match(a, b, widened))
            }),
            _ => true,
        };
        let mut sizes = group.sizes.clone();
        sizes.push(search.people);
        levels_fit && balance_needed(playlist, &sizes).is_some()
    }

    /// What a party in `group` is shown.
    fn status(
        &self,
        group: &Group,
        search: &Search,
        need: usize,
        countdown: Option<&Countdown>,
        now: f64,
    ) -> Status {
        let playlist = &self.playlists[group.playlist];
        let stage = match countdown {
            Some(_) => Stage::WaitingToFill,
            None if group.people >= usize::from(playlist.min) => Stage::Balancing,
            None if group.parties.len() > 1 => Stage::Gathering,
            None => Stage::Searching,
        };
        let seconds = match countdown {
            Some(c) => (c.ends - now).ceil(),
            None => (now - search.since).floor(),
        };
        let widened = if playlist.ranked { group.widened } else { None };
        let (low, high) = level_span(&group.levels, widened);
        Status {
            stage,
            have: group.people as u8,
            need: need as u8,
            seconds: seconds as u16,
            low,
            high,
        }
    }

    /// Make a match of `group`: pick the map and game, split the parties
    /// into teams, add bots, and choose the order to ask PCs to host.
    fn form(&mut self, group: &Group, now: f64) -> Match {
        let Matchmaker {
            playlists,
            searches,
            hosting,
            host_penalties,
            rng,
            ..
        } = self;
        let playlist = &playlists[group.playlist];
        let parties: Vec<&Search> = group.parties.iter().map(|&i| &searches[i]).collect();

        // Any map everyone has, except one the parties just played (unless
        // that's all there is).
        let previous: Vec<&str> = parties
            .iter()
            .filter_map(|s| s.ticket.previous_map.as_deref())
            .collect();
        let fresh: Vec<(usize, u64)> = group
            .maps
            .iter()
            .copied()
            .filter(|&(m, _)| {
                !previous
                    .iter()
                    .any(|p| p.eq_ignore_ascii_case(&playlist.maps[m]))
            })
            .collect();
        let maps = if fresh.is_empty() {
            &group.maps
        } else {
            &fresh
        };
        let (map, hash) = maps[rng.below(maps.len())];
        let variant = playlist.variants[rng.below(playlist.variants.len())].clone();

        let teams = if playlist.teams {
            let level = |s: &&Search| -> u32 {
                let members = s.ticket.members.iter().zip(&s.levels);
                members
                    .map(|(m, &(_, level))| u32::from(level) * m.people() as u32)
                    .sum()
            };
            let levels: Vec<u32> = parties.iter().map(level).collect();
            split(&group.sizes, &levels, rng)
        } else {
            vec![0; parties.len()]
        };
        let red: usize = group
            .sizes
            .iter()
            .zip(&teams)
            .filter(|&(_, &team)| team == 0)
            .map(|(size, _)| size)
            .sum();
        let blue = group.people - red;
        let bots = match playlist.bots {
            Bots::None => 0,
            Bots::Even => usize::from(playlist.teams && red.abs_diff(blue) == 1),
            Bots::Fill(n) => usize::from(n).saturating_sub(group.people),
        };
        let bots = bots.min(MAX_PLAYERS.saturating_sub(group.people)) as u8;

        let mut players = Vec::new();
        let mut pcs = Vec::new();
        for (s, &team) in parties.iter().zip(&teams) {
            for (m, &(level, effective)) in s.ticket.members.iter().zip(&s.levels) {
                players.push(Seat {
                    account: m.account,
                    party: s.ticket.party,
                    team,
                    level,
                    effective,
                    guests: m.guests,
                });
                pcs.push((m.account, m.rtt, s.people));
            }
        }
        let hosts = host_order(&pcs, host_penalties);
        for (account, _, _) in &pcs {
            if let Some(left) = host_penalties.get_mut(account) {
                *left -= 1;
                if *left == 0 {
                    host_penalties.remove(account);
                }
            }
        }
        let id = rng.next();
        let host = hosts[0];
        hosting.push(Hosting {
            id,
            hosts,
            asked: 0,
            deadline: now + HOSTING_TIMEOUT,
        });
        Match {
            id,
            playlist: playlist.id,
            ranked: playlist.ranked,
            map: playlist.maps[map].clone(),
            hash,
            variant,
            bots,
            host,
            players,
        }
    }
}

#[cfg(test)]
mod tests;
