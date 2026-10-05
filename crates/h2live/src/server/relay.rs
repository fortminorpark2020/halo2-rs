//! The relay: games go through the server, so no one has to open a port.
//! For each PC that joins a match (or its party's custom game) the server
//! makes up a pair of tokens, and sends one to that PC and the other to the
//! host (LINK). Each dials `/link` and says its token first (LINK_HELLO);
//! once both of the legs are there, both are told (LINKED), and from then
//! on whatever one sends goes to the other as it is. Since each end has a
//! token of its own, neither can dial the other's leg. A leg whose other
//! end doesn't come within 20 seconds is closed, and so is a link with an
//! end more than 2 MB behind, or whose match or custom game is over.
//!
//! The host also gets a fan-out leg (LINK, to no one): what it sends over
//! that (FANOUT) goes to each of the PCs it names that are linked to it,
//! so what goes to many of them at once is uploaded once. It closes with
//! the match or custom game too.

use super::Server;
use h2net::live::{End, LinkInfo, ToPc, ToServer};
use h2net::Connection;
use std::net::IpAddr;

/// A leg waits this long (seconds) for its other end.
const LEG_WAIT: f64 = 20.0;
/// A link is dropped when either end has this many bytes waiting to go.
const MAX_BEHIND: usize = 2 << 20;
/// Legs one address can have waiting at once: enough for a host with 15
/// PCs joining it and a few more PCs behind the same router.
pub(super) const LEGS_WAITING: usize = 32;
/// An end that sent nothing for this long (seconds) has gone: a game,
/// hosted or joined, sends something every second while it runs.
pub(super) const QUIET: f64 = 5.0;
/// What a fan-out leg says it is, beside the ends of a link (0 for the
/// host's, 1 for the joining PC's).
const FANOUT: usize = 2;

/// A relay leg, waiting for its other end.
pub(super) struct Leg {
    conn: Connection,
    /// Where it came from, and when.
    ip: IpAddr,
    since: f64,
    /// Which leg it said it is (LINK_HELLO): its token, and 0 for the
    /// host's end, 1 for the joining PC's or `FANOUT`.
    end: Option<([u8; 16], usize)>,
}

impl Leg {
    pub(super) fn new(conn: Connection, ip: IpAddr, now: f64) -> Leg {
        Leg {
            conn,
            ip,
            since: now,
            end: None,
        }
    }
}

/// Two legs joined: the host's end and the joining PC's.
pub(super) struct Link {
    ends: [Connection; 2],
    /// The match, or the party whose custom game it's for, and the
    /// accounts at the ends (the host's first).
    id: u64,
    accounts: [u64; 2],
    /// When something last came from each end.
    heard: [f64; 2],
    /// What it was for is over: it closes next time round.
    over: bool,
}

/// A host's fan-out leg.
pub(super) struct Fanout {
    conn: Connection,
    /// The match, or the party whose custom game it's for, and the host.
    id: u64,
    host: u64,
    /// The accounts the last FANOUT named.
    to: Vec<u64>,
    /// What it was for is over: it closes next time round.
    over: bool,
}

/// A link given out (LINK) whose legs haven't both come.
#[derive(Clone, Copy)]
pub(super) struct Token {
    /// The match, or the party whose custom game it's for.
    pub id: u64,
    /// The host's account, then the joining PC's, and the token each
    /// one's leg says.
    pub accounts: [u64; 2],
    pub tokens: [[u8; 16]; 2],
}

impl Server {
    /// The relay's connections open now: legs waiting, links, and fan-out
    /// legs.
    pub(super) fn relay_conns(&self) -> impl Iterator<Item = &Connection> {
        let legs = self.legs.iter().map(|leg| &leg.conn);
        let links = self.links.iter().flat_map(|link| &link.ends);
        legs.chain(links)
            .chain(self.fanouts.iter().map(|f| &f.conn))
    }

    /// Link `joiner` to `host` for match (or custom game) `id` on `map`:
    /// tell both who's at the other end, with the token their leg says.
    /// Each is an account, its level and its team. The host gets a fan-out
    /// leg first, unless it has one.
    pub(super) fn link(&mut self, id: u64, map: &str, host: (u64, u8, u8), joiner: (u64, u8, u8)) {
        self.give_fanout(id, map, host.0);
        let mut tokens = [[0; 16]; 2];
        for token in &mut tokens {
            getrandom::fill(token).expect("random numbers");
        }
        let t = Token {
            id,
            accounts: [host.0, joiner.0],
            tokens,
        };
        for token in tokens {
            self.tokens.insert(token, t);
        }
        for (end, me, (peer, level, team)) in [(0, host, joiner), (1, joiner, host)] {
            let link = LinkInfo {
                token: tokens[end],
                id,
                end: if end == 0 { End::Host } else { End::Joiner },
                peer,
                gamertag: self.gamertag(peer),
                level,
                team,
                map: map.to_string(),
            };
            self.tell(me.0, &ToPc::Link(link));
        }
    }

    /// Give `host` a fan-out leg for match (or custom game) `id` on `map`,
    /// unless it has one, or one given out.
    fn give_fanout(&mut self, id: u64, map: &str, host: u64) {
        let given = self.fanout_tokens.values().any(|&t| t == (id, host));
        let open = self.fanouts.iter().any(|f| (f.id, f.host) == (id, host));
        if given || open {
            return;
        }
        let mut token = [0; 16];
        getrandom::fill(&mut token).expect("random numbers");
        self.fanout_tokens.insert(token, (id, host));
        let link = LinkInfo {
            token,
            id,
            end: End::Fanout,
            peer: 0,
            gamertag: String::new(),
            level: 0,
            team: 0,
            map: map.to_string(),
        };
        self.tell(host, &ToPc::Link(link));
    }

    /// Close the links for `id`, given out or joined, to `account` or (if
    /// `None`) to anyone: what they were for is over. So is the host's
    /// fan-out leg, if it's `account`'s.
    pub(super) fn unlink(&mut self, id: u64, account: Option<u64>) {
        let theirs = |link: u64, accounts: &[u64; 2]| {
            link == id && account.is_none_or(|a| accounts.contains(&a))
        };
        self.tokens.retain(|_, t| !theirs(t.id, &t.accounts));
        for l in &mut self.links {
            l.over |= theirs(l.id, &l.accounts);
        }
        let hosts = |link: u64, host: u64| link == id && account.is_none_or(|a| a == host);
        self.fanout_tokens.retain(|_, t| !hosts(t.0, t.1));
        for f in &mut self.fanouts {
            f.over |= hosts(f.id, f.host);
        }
    }

    /// `account` has a link for `id`, given out or joined.
    pub(super) fn has_link(&self, id: u64, account: u64) -> bool {
        let theirs = |link: u64, accounts: &[u64; 2]| link == id && accounts.contains(&account);
        self.tokens.values().any(|t| theirs(t.id, &t.accounts))
            || self
                .links
                .iter()
                .any(|l| !l.over && theirs(l.id, &l.accounts))
    }

    fn gamertag(&self, account: u64) -> String {
        self.accounts
            .get(&account)
            .map_or(String::new(), |a| a.gamertag.clone())
    }

    /// A new leg from `ip`, unless that address has too many waiting.
    pub(super) fn add_leg(&mut self, conn: Connection, ip: IpAddr, now: f64) {
        if self.legs.iter().filter(|l| l.ip == ip).count() < LEGS_WAITING {
            self.legs.push(Leg::new(conn, ip, now));
        }
    }

    /// Relay legs say which they are, then wait for their other end, and
    /// are joined to it once it's there; a fan-out leg is taken at once.
    /// A leg that says anything else first, names a link that wasn't given
    /// out (or the other end of one), or waits too long is dropped.
    pub(super) fn read_legs(&mut self, now: f64) {
        let (tokens, fanout_tokens) = (&self.tokens, &self.fanout_tokens);
        self.legs.retain_mut(|leg| {
            let Ok(messages) = leg.conn.receive() else {
                return false;
            };
            for (kind, body) in messages {
                let Ok(ToServer::LinkHello { token, account }) = ToServer::read(kind, &body) else {
                    return false;
                };
                let end = tokens.get(&token).and_then(|t| {
                    let end = t.tokens.iter().position(|&x| x == token)?;
                    (t.accounts[end] == account).then_some(end)
                });
                let fanout = fanout_tokens.get(&token).filter(|t| t.1 == account);
                match end.or(fanout.map(|_| FANOUT)) {
                    Some(end) if leg.end.is_none() => leg.end = Some((token, end)),
                    _ => return false,
                }
            }
            now - leg.since < LEG_WAIT
        });
        let mut k = 0;
        while k < self.legs.len() {
            let Some((token, end)) = self.legs[k].end else {
                k += 1;
                continue;
            };
            if end == FANOUT {
                let leg = self.legs.remove(k);
                self.add_fanout(token, leg.conn);
                continue;
            }
            let other = self
                .tokens
                .get(&token)
                .map(|t| (t.tokens[1 - end], 1 - end));
            let Some(j) = other.and_then(|o| self.legs.iter().position(|l| l.end == Some(o)))
            else {
                k += 1;
                continue;
            };
            let (first, second) = (k.max(j), k.min(j));
            let a = self.legs.remove(first);
            let b = self.legs.remove(second);
            let (host, joiner) = if a.end.is_some_and(|(_, end)| end == 0) {
                (a, b)
            } else {
                (b, a)
            };
            self.join(token, host.conn, joiner.conn, now);
            k = 0;
        }
    }

    /// Both legs of the link `token` names are here: tell them, and start
    /// passing what each sends to the other.
    fn join(&mut self, token: [u8; 16], host: Connection, joiner: Connection, now: f64) {
        let Some(t) = self.tokens.get(&token).copied() else {
            return;
        };
        for token in t.tokens {
            self.tokens.remove(&token);
        }
        let mut ends = [host, joiner];
        for end in &mut ends {
            ToPc::Linked.send(end);
            // A failure shows up when relaying, next time.
            let _ = end.flush();
        }
        self.links.push(Link {
            ends,
            id: t.id,
            accounts: t.accounts,
            heard: [now; 2],
            over: false,
        });
        self.linked(t.id, t.accounts[1]);
    }

    /// The host's fan-out leg `token` names is here: tell it, and start
    /// passing on what it sends.
    fn add_fanout(&mut self, token: [u8; 16], mut conn: Connection) {
        let Some((id, host)) = self.fanout_tokens.remove(&token) else {
            return;
        };
        ToPc::Linked.send(&mut conn);
        // A failure shows up when reading it, next time.
        let _ = conn.flush();
        self.fanouts.push(Fanout {
            conn,
            id,
            host,
            to: Vec::new(),
            over: false,
        });
    }

    /// Pass what came over each fan-out leg on to each PC it names that's
    /// linked to that host, counting the bytes (they go out with the
    /// links' own, in `relay`); the host is heard from on all its links. A
    /// fan-out leg that sends anything else, or whose match is over,
    /// closes.
    pub(super) fn fan_out(&mut self, now: f64) {
        let mut k = 0;
        while k < self.fanouts.len() {
            let f = &mut self.fanouts[k];
            let mut open = !f.over;
            let messages = match f.conn.receive() {
                Ok(messages) if open => messages,
                _ => {
                    open = false;
                    Vec::new()
                }
            };
            if !messages.is_empty() {
                let theirs = |l: &&mut Link| (l.id, l.accounts[0]) == (f.id, f.host);
                for link in self.links.iter_mut().filter(theirs) {
                    link.heard[0] = now;
                }
            }
            let mut bytes = 0;
            for (kind, body) in messages {
                let Ok(ToServer::Fanout { to, kind, body }) = ToServer::read(kind, &body) else {
                    open = false;
                    break;
                };
                if let Some(to) = to {
                    f.to = to;
                }
                let theirs = self.links.iter_mut().filter(|l| {
                    let (id, host, joiner) = (l.id, l.accounts[0], l.accounts[1]);
                    !l.over && (id, host) == (f.id, f.host) && f.to.contains(&joiner)
                });
                for link in theirs {
                    link.ends[1].send(kind, &body);
                    bytes += 1 + body.len() as u64;
                }
            }
            if let Some(relayed) = self.relayed.get_mut(&f.id) {
                *relayed += bytes;
            }
            open &= f.conn.flush().is_ok();
            if open {
                k += 1;
            } else {
                let f = self.fanouts.remove(k);
                self.linger(f.conn, now);
            }
        }
    }

    /// Pass on what each end of each link sent, counting the bytes. A link
    /// is dropped when an end goes, or falls too far behind. If that's the
    /// host's end (unless the joining PC had gone quiet: the host gave up
    /// on it), or the joining PC's end with the host's gone quiet, the
    /// match hears the joining PC lost the host. Links that are over
    /// close, after a moment (so the ends hear why from the server first).
    pub(super) fn relay(&mut self, now: f64) {
        let mut dropped = Vec::new();
        let mut k = 0;
        while k < self.links.len() {
            if self.links[k].over {
                let link = self.links.remove(k);
                for end in link.ends {
                    self.linger(end, now);
                }
                continue;
            }
            let link = &mut self.links[k];
            // The end that broke the link, if one did.
            let mut broke = None;
            for from in 0..2 {
                let Ok(messages) = link.ends[from].receive() else {
                    broke = Some(from);
                    break;
                };
                if !messages.is_empty() {
                    link.heard[from] = now;
                }
                let bytes: usize = messages.iter().map(|(_, body)| 1 + body.len()).sum();
                if let Some(relayed) = self.relayed.get_mut(&link.id) {
                    *relayed += bytes as u64;
                }
                for (kind, body) in messages {
                    link.ends[1 - from].send(kind, &body);
                }
            }
            for (i, end) in link.ends.iter_mut().enumerate() {
                let open = end.flush().is_ok() && end.backlog() <= MAX_BEHIND;
                if !open && broke.is_none() {
                    broke = Some(i);
                }
            }
            match broke {
                None => k += 1,
                Some(end) => dropped.push((self.links.remove(k), end)),
            }
        }
        for (link, end) in dropped {
            let quiet = |end: usize| now - link.heard[end] >= QUIET;
            if end == 0 && !quiet(1) || end == 1 && quiet(0) {
                self.host_dropped(link.id, link.accounts[1]);
            }
            // What's left to send still goes, for a moment.
            for end in link.ends {
                self.linger(end, now);
            }
        }
    }

    /// Bytes relayed so far for match (or custom game) `id`, while it's on.
    pub fn relayed(&self, id: u64) -> u64 {
        self.relayed.get(&id).copied().unwrap_or(0)
    }
}
