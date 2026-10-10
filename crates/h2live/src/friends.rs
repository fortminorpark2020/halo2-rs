//! Friends lists, like Xbox Live 1.0's: who is friends with whom, and the
//! friend requests waiting for an answer, with the rules for changing them.
//! Nothing here reads the network or a clock; the server passes the time
//! a request is made, and keeps the lists in `friends.txt` in its data
//! folder, written whole whenever they change:
//!
//! ```text
//! f <id> <id>
//! r <from id> <to id> <unix time>
//! ```
//!
//! Ids are 16 hex digits, as in `accounts.txt`. An `f` line is a
//! friendship, the smaller id first; an `r` line is a request waiting for
//! an answer, with when it was sent, the oldest first. Empty lines are
//! allowed. Ids that `accounts.txt` doesn't have are kept (a lost data
//! folder may come back from stat cards). No file is an empty list.
//!
//! The rules: friends and the requests a player sent count together
//! towards `MAX_FRIENDS` (Xbox Live 1.0 kept 100 friends; counting the
//! sent requests in is our own choice), and a player has at most
//! `MAX_FRIEND_REQUESTS` waiting for their answer. Asking someone who
//! already asked you makes you friends at once. Requests don't expire.
//! The limits are checked when players act, not when loading.

use crate::store;
use h2net::live::{MAX_FRIENDS, MAX_FRIEND_REQUESTS};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

/// Every friendship and waiting request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Friends {
    /// Each player's friends, kept both ways (b is in a's set and a in
    /// b's), so a player's friends are a lookup.
    friends: HashMap<u64, BTreeSet<u64>>,
    /// Requests waiting for an answer, the oldest first.
    requests: Vec<Request>,
}

/// A friend request waiting for an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    pub from: u64,
    pub to: u64,
    /// When it was sent (Unix time).
    pub unix: u64,
}

/// What came of a friend request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asked {
    /// It waits for an answer.
    Sent,
    /// The other had asked us already: now we're friends.
    NowFriends,
}

/// Why a friend request or an accept was turned down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Yourself,
    AlreadyFriends,
    AlreadyAsked,
    /// The one asking or accepting has no room for one more.
    ListFull,
    /// The one asked has too many requests waiting.
    TheirRequestsFull,
}

impl Friends {
    /// Friends as `text` writes them.
    pub fn parse(text: &str) -> Result<Friends, String> {
        let mut friends = Friends::default();
        for (i, line) in text.lines().enumerate() {
            let at = |e: &str| format!("line {}: {e}", i + 1);
            let words: Vec<&str> = line.split(' ').collect();
            match words[..] {
                ["f", a, b] => {
                    let (a, b) = (id(a), id(b));
                    let (a, b) = a.zip(b).ok_or_else(|| at("bad id"))?;
                    if a == b {
                        return Err(at("a player paired with themselves"));
                    }
                    if friends.are_friends(a, b) {
                        return Err(at("a friendship listed twice"));
                    }
                    friends.befriend(a, b);
                }
                ["r", from, to, unix] => {
                    let (from, to) = id(from).zip(id(to)).ok_or_else(|| at("bad id"))?;
                    let unix = unix.parse().map_err(|_| at("bad time"))?;
                    let request = Request { from, to, unix };
                    if request.from == request.to {
                        return Err(at("a player paired with themselves"));
                    }
                    let between = |r: &Request| {
                        (r.from, r.to) == (request.from, request.to)
                            || (r.from, r.to) == (request.to, request.from)
                    };
                    if friends.requests.iter().any(between) {
                        return Err(at("a request listed twice, or both ways"));
                    }
                    friends.requests.push(request);
                }
                _ if line.trim().is_empty() => {}
                _ => return Err(at("not a friendship or a request")),
            }
        }
        // A request between friends would have been an accept.
        if let Some(r) = friends
            .requests
            .iter()
            .find(|r| friends.are_friends(r.from, r.to))
        {
            return Err(format!(
                "a request between friends ({:016x} and {:016x})",
                r.from, r.to
            ));
        }
        Ok(friends)
    }

    /// The lines of friends.txt: friendships (the smaller id first, in
    /// order), then requests, the oldest first.
    pub fn text(&self) -> String {
        let mut pairs: Vec<(u64, u64)> = self
            .friends
            .iter()
            .flat_map(|(&a, set)| set.iter().filter(move |&&b| a < b).map(move |&b| (a, b)))
            .collect();
        pairs.sort_unstable();
        let mut text = String::new();
        for (a, b) in pairs {
            text += &format!("f {a:016x} {b:016x}\n");
        }
        for r in &self.requests {
            text += &format!("r {:016x} {:016x} {}\n", r.from, r.to, r.unix);
        }
        text
    }

    /// The friends in `path`; an empty list if there's no such file.
    pub fn load(path: &Path) -> Result<Friends, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Friends::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Friends::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Write them to `path` (friends.txt) whole.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        store::replace(path, &self.text())
    }

    pub fn are_friends(&self, a: u64, b: u64) -> bool {
        self.friends.get(&a).is_some_and(|set| set.contains(&b))
    }

    /// `a`'s friends, in order of id.
    pub fn friends_of(&self, a: u64) -> Vec<u64> {
        self.friends
            .get(&a)
            .map_or(Vec::new(), |set| set.iter().copied().collect())
    }

    /// Who asked `a` to be friends, the oldest request first.
    pub fn asked_by(&self, a: u64) -> Vec<u64> {
        let to_a = self.requests.iter().filter(|r| r.to == a);
        to_a.map(|r| r.from).collect()
    }

    /// Who `a` asked to be friends, the oldest request first.
    pub fn asking(&self, a: u64) -> Vec<u64> {
        let from_a = self.requests.iter().filter(|r| r.from == a);
        from_a.map(|r| r.to).collect()
    }

    /// `a`'s friends and the requests `a` sent: what `MAX_FRIENDS` limits.
    pub fn count(&self, a: u64) -> usize {
        let friends = self.friends.get(&a).map_or(0, BTreeSet::len);
        friends + self.requests.iter().filter(|r| r.from == a).count()
    }

    /// `from` asks `to` to be friends, at Unix time `unix`.
    pub fn ask(&mut self, from: u64, to: u64, unix: u64) -> Result<Asked, Refusal> {
        if from == to {
            return Err(Refusal::Yourself);
        }
        if self.are_friends(from, to) {
            return Err(Refusal::AlreadyFriends);
        }
        if self.waiting(from, to).is_some() {
            return Err(Refusal::AlreadyAsked);
        }
        if self.count(from) >= MAX_FRIENDS {
            return Err(Refusal::ListFull);
        }
        // They asked us first: as if we accepted. (Their request becomes a
        // friend, so their count is as it was.)
        if let Some(i) = self.waiting(to, from) {
            self.requests.remove(i);
            self.befriend(from, to);
            return Ok(Asked::NowFriends);
        }
        if self.requests.iter().filter(|r| r.to == to).count() >= MAX_FRIEND_REQUESTS {
            return Err(Refusal::TheirRequestsFull);
        }
        self.requests.push(Request { from, to, unix });
        Ok(Asked::Sent)
    }

    /// `me` accepts the request from `from`: true if there was one, false
    /// if there's no such request (taken back a moment before, say). `me`
    /// needs room for one more; the asker's count doesn't change.
    pub fn accept(&mut self, me: u64, from: u64) -> Result<bool, Refusal> {
        let Some(i) = self.waiting(from, me) else {
            return Ok(false);
        };
        if self.count(me) >= MAX_FRIENDS {
            return Err(Refusal::ListFull);
        }
        self.requests.remove(i);
        self.befriend(me, from);
        Ok(true)
    }

    /// `me` turns down the request from `from`. True if there was one.
    pub fn decline(&mut self, me: u64, from: u64) -> bool {
        let Some(i) = self.waiting(from, me) else {
            return false;
        };
        self.requests.remove(i);
        true
    }

    /// `me` stops being friends with `other`, or takes back the request
    /// they sent `other`. True if either was so.
    pub fn remove(&mut self, me: u64, other: u64) -> bool {
        if self.are_friends(me, other) {
            for (a, b) in [(me, other), (other, me)] {
                if let Some(set) = self.friends.get_mut(&a) {
                    set.remove(&b);
                    if set.is_empty() {
                        self.friends.remove(&a);
                    }
                }
            }
            return true;
        }
        match self.waiting(me, other) {
            Some(i) => {
                self.requests.remove(i);
                true
            }
            None => false,
        }
    }

    /// Where the request from `from` to `to` is in the list, if it's there.
    fn waiting(&self, from: u64, to: u64) -> Option<usize> {
        self.requests
            .iter()
            .position(|r| r.from == from && r.to == to)
    }

    fn befriend(&mut self, a: u64, b: u64) {
        self.friends.entry(a).or_default().insert(b);
        self.friends.entry(b).or_default().insert(a);
    }
}

/// An id written as 16 hex digits.
fn id(text: &str) -> Option<u64> {
    store::unhex::<8>(text).map(u64::from_be_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friends_read_back_as_written() {
        let mut f = Friends::default();
        assert_eq!(f.ask(3, 1, 100), Ok(Asked::Sent));
        assert_eq!(f.ask(1, 2, 101), Ok(Asked::Sent));
        assert_eq!(f.accept(2, 1), Ok(true));
        assert_eq!(f.ask(4, 1, 102), Ok(Asked::Sent));
        assert_eq!(f.ask(0xffff_0000_0000_0001, 2, 103), Ok(Asked::Sent));
        let text = f.text();
        assert_eq!(
            text,
            "f 0000000000000001 0000000000000002\n\
             r 0000000000000003 0000000000000001 100\n\
             r 0000000000000004 0000000000000001 102\n\
             r ffff000000000001 0000000000000002 103\n"
        );
        assert_eq!(Friends::parse(&text), Ok(f.clone()));
        // Empty lines are no matter.
        assert_eq!(Friends::parse(&format!("\n{text}\n")), Ok(f));
        assert_eq!(Friends::parse(""), Ok(Friends::default()));
    }

    #[test]
    fn no_file_is_an_empty_list_and_saving_replaces_it() {
        let dir = std::env::temp_dir().join(format!("h2live-friends-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("friends.txt");
        assert_eq!(Friends::load(&path), Ok(Friends::default()));
        let mut f = Friends::default();
        f.ask(1, 2, 5).unwrap();
        f.save(&path).unwrap();
        assert_eq!(Friends::load(&path), Ok(f.clone()));
        f.remove(1, 2);
        f.save(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        // Nothing is left beside it.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::write(&path, "hello\n").unwrap();
        assert!(Friends::load(&path).unwrap_err().contains("friends.txt"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn broken_friends_files_are_errors() {
        let a = "0000000000000001";
        let b = "0000000000000002";
        for bad in [
            // Another letter, bad hex, or the wrong number of words.
            format!("g {a} {b}\n"),
            format!("f {a} 00000000000000zz\n"),
            format!("f {a} 1\n"),
            format!("f {a}\n"),
            format!("f {a} {b} 3\n"),
            format!("r {a} {b}\n"),
            format!("r {a} {b} -1\n"),
            format!("r {a} {b} soon\n"),
            // An id paired with itself.
            format!("f {a} {a}\n"),
            format!("r {a} {a} 1\n"),
            // Listed twice, either way round.
            format!("f {a} {b}\nf {a} {b}\n"),
            format!("f {a} {b}\nf {b} {a}\n"),
            format!("r {a} {b} 1\nr {a} {b} 2\n"),
            // Requests both ways, or between friends.
            format!("r {a} {b} 1\nr {b} {a} 2\n"),
            format!("f {a} {b}\nr {a} {b} 1\n"),
            format!("r {b} {a} 1\nf {a} {b}\n"),
            // Spaces where there shouldn't be.
            format!(" f {a} {b}\n"),
            format!("f  {a} {b}\n"),
        ] {
            assert!(Friends::parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn asking_follows_the_rules() {
        let mut f = Friends::default();
        assert_eq!(f.ask(1, 1, 0), Err(Refusal::Yourself));
        assert_eq!(f.ask(1, 2, 0), Ok(Asked::Sent));
        assert_eq!(f.ask(1, 2, 1), Err(Refusal::AlreadyAsked));
        assert_eq!((f.count(1), f.count(2)), (1, 0));
        // Asked both ways: friends at once, the other's count as it was.
        assert_eq!(f.ask(2, 1, 2), Ok(Asked::NowFriends));
        assert!(f.are_friends(1, 2) && f.are_friends(2, 1));
        assert_eq!((f.count(1), f.count(2)), (1, 1));
        assert!(f.asked_by(1).is_empty() && f.asking(1).is_empty());
        assert_eq!(f.ask(1, 2, 3), Err(Refusal::AlreadyFriends));
        assert_eq!(f.ask(2, 1, 3), Err(Refusal::AlreadyFriends));
        assert_eq!(f.friends_of(1), [2]);
        assert_eq!(f.friends_of(3), Vec::<u64>::new());
    }

    #[test]
    fn friends_and_requests_sent_count_together_to_a_hundred() {
        let mut f = Friends::default();
        // 60 friends and 40 requests sent.
        for other in 100..160 {
            f.ask(other, 1, 0).unwrap();
            assert_eq!(f.accept(1, other), Ok(true));
        }
        for other in 200..240 {
            assert_eq!(f.ask(1, other, 0), Ok(Asked::Sent));
        }
        assert_eq!(f.count(1), MAX_FRIENDS);
        assert_eq!(f.ask(1, 300, 0), Err(Refusal::ListFull));
        // Someone who asked can't be accepted, nor asked back, without
        // room; they can still ask.
        assert_eq!(f.ask(301, 1, 0), Ok(Asked::Sent));
        assert_eq!(f.accept(1, 301), Err(Refusal::ListFull));
        assert_eq!(f.ask(1, 301, 0), Err(Refusal::ListFull));
        assert_eq!(f.asked_by(1), [301]);
        // Taking a request back makes room.
        assert!(f.remove(1, 239));
        assert_eq!(f.accept(1, 301), Ok(true));
        assert_eq!(f.count(1), MAX_FRIENDS);
        // Accepting doesn't change the asker's count.
        assert_eq!(f.count(301), 1);
    }

    #[test]
    fn a_hundred_requests_wait_at_most() {
        let mut f = Friends::default();
        for other in 10..10 + MAX_FRIEND_REQUESTS as u64 {
            assert_eq!(f.ask(other, 1, other), Ok(Asked::Sent));
        }
        assert_eq!(f.ask(5, 1, 0), Err(Refusal::TheirRequestsFull));
        // They're kept oldest first.
        assert_eq!(f.asked_by(1)[..3], [10, 11, 12]);
        assert_eq!(f.asking(10), [1]);
        // One answered makes room for another.
        assert!(f.decline(1, 50));
        assert_eq!(f.ask(5, 1, 0), Ok(Asked::Sent));
        assert_eq!(f.asked_by(1).last(), Some(&5));
    }

    #[test]
    fn answers_and_removals_say_whether_they_changed_anything() {
        let mut f = Friends::default();
        f.ask(1, 2, 0).unwrap();
        f.ask(3, 2, 1).unwrap();
        f.ask(2, 4, 2).unwrap();
        // Only a waiting request can be answered.
        assert_eq!(f.accept(1, 2), Ok(false));
        assert!(!f.decline(1, 2));
        assert!(f.decline(2, 3));
        assert!(!f.decline(2, 3));
        assert_eq!(f.accept(2, 1), Ok(true));
        assert_eq!(f.accept(2, 1), Ok(false));
        // Removing a friend, or a request sent; not one received.
        assert!(!f.remove(4, 2));
        assert!(f.remove(2, 4));
        assert!(!f.remove(2, 4));
        assert!(f.remove(1, 2));
        assert!(!f.are_friends(2, 1));
        assert!(!f.remove(2, 1));
        assert_eq!(f, Friends::default());
        assert_eq!(f.text(), "");
    }
}
