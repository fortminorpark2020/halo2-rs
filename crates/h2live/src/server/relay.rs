//! The relay: games go through the server, so no one has to open a port.
//! For each PC that joins a match (or its party's custom game) the server
//! makes up a pair of tokens, and sends one to that PC and the other to the
//! host (LINK). Each dials `/link` and says its token first (LINK_HELLO);
//! once both of the legs are there, both are told (LINKED), and from then
//! on whatever one sends goes to the other as it is. Since each end has a
//! token of its own, neither can dial the other's leg. A leg whose other
//! end doesn't come within 20 seconds is closed, and so is a link with an
//! end more than 2 MB behind, or whose match or custom game is over.

use super::Server;
use h2net::live::{LinkInfo, ToPc, ToServer};
use h2net::Connection;
use std::net::IpAddr;

/// A leg waits this long (seconds) for its other end.
const LEG_WAIT: f64 = 20.0;
/// A link is dropped when either end has this many bytes waiting to go.
const MAX_BEHIND: usize = 2 << 20;
/// Legs one address can have waiting at once: enough for a host with 15
/// PCs joining it and a few more PCs behind the same router.
pub(super) const LEGS_WAITING: usize = 32;
/// A host's end that sent nothing for this long (seconds) has gone: its
/// game sends something every second while it runs.
const HOST_QUIET: f64 = 5.0;

/// A relay leg, waiting for its other end.
pub(super) struct Leg {
    conn: Connection,
    /// Where it came from, and when.
    ip: IpAddr,
    since: f64,
    /// Which leg it said it is (LINK_HELLO): its token, and 0 for the
    /// host's end or 1 for the joining PC's.
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
    /// Link `joiner` to `host` for match (or custom game) `id` on `map`:
    /// tell both who's at the other end, with the token their leg says.
    /// Each is an account, its level and its team.
    pub(super) fn link(&mut self, id: u64, map: &str, host: (u64, u8, u8), joiner: (u64, u8, u8)) {
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
                joiner: end == 1,
                peer,
                gamertag: self.gamertag(peer),
                level,
                team,
                map: map.to_string(),
            };
            self.tell(me.0, &ToPc::Link(link));
        }
    }

    /// Close the links for `id`, given out or joined, to `account` or (if
    /// `None`) to anyone: what they were for is over.
    pub(super) fn unlink(&mut self, id: u64, account: Option<u64>) {
        let theirs = |link: u64, accounts: &[u64; 2]| {
            link == id && account.is_none_or(|a| accounts.contains(&a))
        };
        self.tokens.retain(|_, t| !theirs(t.id, &t.accounts));
        self.links.retain(|l| !theirs(l.id, &l.accounts));
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
    /// are joined to it once it's there. A leg that says anything else
    /// first, names a link that wasn't given out (or the other end of
    /// one), or waits too long is dropped.
    pub(super) fn read_legs(&mut self, now: f64) {
        let tokens = &self.tokens;
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
                match end {
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
        });
        self.linked(t.id, t.accounts[1]);
    }

    /// Pass on what each end of each link sent, counting the bytes. A link
    /// is dropped when an end goes, or falls too far behind; if that's the
    /// host's end (or it had gone quiet), the match hears the joining PC
    /// lost the host.
    pub(super) fn relay(&mut self, now: f64) {
        let mut dropped = Vec::new();
        let mut k = 0;
        while k < self.links.len() {
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
            if end == 0 || now - link.heard[0] >= HOST_QUIET {
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
