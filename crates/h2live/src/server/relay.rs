//! The relay: games go through the server, so no one has to open a port.
//! For each PC that joins a match (or its party's custom game) the server
//! makes up a token and sends it to that PC and to the host (LINK). Each
//! dials `/link` and says the token first (LINK_HELLO); once both of the
//! legs are there, both are told (LINKED), and from then on whatever one
//! sends goes to the other as it is. A leg whose other end doesn't come
//! within 20 seconds is closed, and so is a link with an end more than
//! 2 MB behind.

use super::Server;
use h2net::live::{LinkInfo, ToPc, ToServer};
use h2net::Connection;

/// A leg waits this long (seconds) for its other end.
const LEG_WAIT: f64 = 20.0;
/// A link is dropped when either end has this many bytes waiting to go.
const MAX_BEHIND: usize = 2 << 20;

/// A relay leg, waiting for its other end.
pub(super) struct Leg {
    conn: Connection,
    since: f64,
    /// Which leg it said it is (LINK_HELLO): its token, and 0 for the
    /// host's end or 1 for the joining PC's.
    end: Option<([u8; 16], usize)>,
}

impl Leg {
    pub(super) fn new(conn: Connection, now: f64) -> Leg {
        Leg {
            conn,
            since: now,
            end: None,
        }
    }
}

/// Two legs joined: the host's end and the joining PC's.
pub(super) struct Link {
    ends: [Connection; 2],
    /// The match, or the party whose custom game it's for.
    id: u64,
}

/// A link given out (LINK) whose legs haven't both come.
pub(super) struct Token {
    /// The match, or the party whose custom game it's for.
    pub id: u64,
    /// The host's account, then the joining PC's.
    pub accounts: [u64; 2],
}

impl Server {
    /// Link `joiner` to `host` for match (or custom game) `id` on `map`:
    /// tell both who's at the other end, with the token their legs say.
    /// Each is an account, its level and its team.
    pub(super) fn link(&mut self, id: u64, map: &str, host: (u64, u8, u8), joiner: (u64, u8, u8)) {
        let mut token = [0; 16];
        getrandom::fill(&mut token).expect("random numbers");
        self.tokens.insert(
            token,
            Token {
                id,
                accounts: [host.0, joiner.0],
            },
        );
        for (me, (peer, level, team), joining) in [(host, joiner, false), (joiner, host, true)] {
            let link = LinkInfo {
                token,
                id,
                joiner: joining,
                peer,
                gamertag: self.gamertag(peer),
                level,
                team,
                map: map.to_string(),
            };
            self.tell(me.0, &ToPc::Link(link));
        }
    }

    /// Forget the links given out for `id` that haven't come, to `account`
    /// or (if `None`) to anyone.
    pub(super) fn unlink(&mut self, id: u64, account: Option<u64>) {
        self.tokens
            .retain(|_, t| t.id != id || account.is_some_and(|a| !t.accounts.contains(&a)));
    }

    fn gamertag(&self, account: u64) -> String {
        self.accounts
            .get(&account)
            .map_or(String::new(), |a| a.gamertag.clone())
    }

    /// Relay legs say which they are, then wait for their other end, and
    /// are joined to it once it's there. A leg that says anything else
    /// first, names a link that wasn't given out, or waits too long is
    /// dropped.
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
                let end = tokens
                    .get(&token)
                    .and_then(|t| t.accounts.iter().position(|&a| a == account));
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
            let Some(j) = self
                .legs
                .iter()
                .position(|l| l.end == Some((token, 1 - end)))
            else {
                k += 1;
                continue;
            };
            let (first, second) = (k.max(j), k.min(j));
            let a = self.legs.remove(first);
            let b = self.legs.remove(second);
            let (host, joiner) = if a.end == Some((token, 0)) {
                (a, b)
            } else {
                (b, a)
            };
            self.join(token, host.conn, joiner.conn);
            k = 0;
        }
    }

    /// Both legs of `token` are here: tell them, and start passing what
    /// each sends to the other.
    fn join(&mut self, token: [u8; 16], host: Connection, joiner: Connection) {
        let Some(t) = self.tokens.remove(&token) else {
            return;
        };
        let mut ends = [host, joiner];
        for end in &mut ends {
            ToPc::Linked.send(end);
            // A failure shows up when relaying, next time.
            let _ = end.flush();
        }
        self.links.push(Link { ends, id: t.id });
        self.linked(t.id, t.accounts[1]);
    }

    /// Pass on what each end of each link sent, counting the bytes. A link
    /// is dropped when an end goes, or falls too far behind.
    pub(super) fn relay(&mut self, now: f64) {
        let mut dropped = Vec::new();
        let mut k = 0;
        while k < self.links.len() {
            let link = &mut self.links[k];
            let mut open = true;
            for from in 0..2 {
                let Ok(messages) = link.ends[from].receive() else {
                    open = false;
                    break;
                };
                let bytes: usize = messages.iter().map(|(_, body)| 1 + body.len()).sum();
                if let Some(relayed) = self.relayed.get_mut(&link.id) {
                    *relayed += bytes as u64;
                }
                for (kind, body) in messages {
                    link.ends[1 - from].send(kind, &body);
                }
            }
            for end in &mut link.ends {
                open &= end.flush().is_ok() && end.backlog() <= MAX_BEHIND;
            }
            if open {
                k += 1;
            } else {
                dropped.push(self.links.remove(k));
            }
        }
        // What's left to send still goes, for a moment.
        for link in dropped {
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
