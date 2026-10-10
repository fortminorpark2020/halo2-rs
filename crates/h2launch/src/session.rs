//! A networked match: the machines, the players and the relay, read from a
//! session file that every launcher in the match gets (`--session`), and
//! written into the game options the way MCC describes a session to the
//! engine (networking research 3.1 and 3.5).
//!
//! ```text
//! # Two launchers on one PC through h2relay on port 47051.
//! relay = 127.0.0.1:47051
//! room = 0x5EED000000000001
//! secure = 0x1234567890ABCDEF
//! host = 0
//! machine = 0x4832000000000001
//! machine = 0x4832000000000002
//! player = 0x0009000000000001 machine=0 team=0 name=Host
//! player = 0x0009000000000002 machine=1 team=1 name=Guest
//! ```
//!
//! Each launcher is told which machine it is with `--me <index>` (or a
//! `me = <index>` line). The first player on that machine is the local
//! player: the profile, input and `--xuid` belong to them.
//!
//! Every launcher writes the same roster, the way an MCC matchmade game
//! knows all its players before it loads; only the flags (host, or a
//! machine that searches for the host's game and joins it) and the local
//! network id differ. The engine's meaning of these fields is
//! partly inferred (research 3.5): which values work is what the test
//! stages find out, and `--set-option` can still override any of them.

use crate::options::{self, off, player, GameOptions, PEER_SLOTS, PLAYER_SLOTS, PLAYER_STRIDE};
use std::net::{SocketAddr, ToSocketAddrs};

/// The relay's "every member" id; a machine can't have it.
const BROADCAST: u64 = u64::MAX;

#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    pub xuid: u64,
    /// Index into `Session::machines`.
    pub machine: usize,
    pub team: i32,
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    /// The relay server, as written (resolved by `relay_addr`).
    pub relay: String,
    /// The match's room token on the relay.
    pub room: u64,
    /// This launcher's member key for a room h2live issued (hex), if any.
    pub key: Option<String>,
    /// Written to 0x50 on every machine.
    pub secure: u64,
    /// Index of the hosting machine.
    pub host: usize,
    /// Network ids, one per machine, in the order of the engine's address
    /// list.
    pub machines: Vec<u64>,
    pub players: Vec<Player>,
    /// Written to 0x0B ("compared with the live peer count") when given.
    pub threshold: Option<u8>,
    /// This launcher's machine, if the file says.
    pub me: Option<usize>,
}

fn number(line: usize, what: &str, v: &str) -> Result<u64, String> {
    crate::util::parse_u64(v.trim())
        .ok_or_else(|| format!("line {line}: {what} {v:?} is not a number"))
}

fn index(line: usize, what: &str, v: &str) -> Result<usize, String> {
    let n = number(line, what, v)?;
    usize::try_from(n).map_err(|_| format!("line {line}: {what} {v:?} is too big"))
}

impl Session {
    /// Reads a session file's text. Blank lines and `#` comments are
    /// skipped; every other line is `key = value`.
    pub fn parse(text: &str) -> Result<Session, String> {
        let mut s = Session {
            relay: String::new(),
            room: 0,
            key: None,
            secure: 0,
            host: 0,
            machines: Vec::new(),
            players: Vec::new(),
            threshold: None,
            me: None,
        };
        let mut have_room = false;
        for (i, raw) in text.lines().enumerate() {
            let n = i + 1;
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let (k, v) = line
                .split_once('=')
                .ok_or_else(|| format!("line {n}: expected key = value, got {line:?}"))?;
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            match k.as_str() {
                "relay" => s.relay = v.to_string(),
                "room" => {
                    s.room = number(n, "room", v)?;
                    have_room = true;
                }
                "key" => s.key = Some(v.to_string()),
                "secure" => s.secure = number(n, "secure", v)?,
                "host" => s.host = index(n, "host", v)?,
                "me" => s.me = Some(index(n, "me", v)?),
                "threshold" => {
                    let t = number(n, "threshold", v)?;
                    s.threshold = Some(
                        u8::try_from(t)
                            .map_err(|_| format!("line {n}: threshold {t} is over 255"))?,
                    );
                }
                "machine" => s.machines.push(number(n, "machine", v)?),
                "player" => s.players.push(Self::player(n, v)?),
                other => return Err(format!("line {n}: unknown key {other:?}")),
            }
        }
        if s.relay.is_empty() {
            return Err("no relay = line".into());
        }
        if !have_room {
            return Err("no room = line".into());
        }
        s.check()?;
        Ok(s)
    }

    /// `player = <xuid> machine=<i> [team=<t>] [name=<gamertag>]`.
    fn player(n: usize, v: &str) -> Result<Player, String> {
        let mut words = v.split_whitespace();
        let xuid = number(n, "player xuid", words.next().unwrap_or(""))?;
        let mut p = Player {
            xuid,
            machine: usize::MAX,
            team: 0,
            name: None,
        };
        for w in words {
            let (k, val) = w
                .split_once('=')
                .ok_or_else(|| format!("line {n}: expected name=value, got {w:?}"))?;
            match k.to_ascii_lowercase().as_str() {
                "machine" => p.machine = index(n, "machine", val)?,
                "team" => {
                    p.team = crate::util::parse_i128(val)
                        .and_then(|t| i32::try_from(t).ok())
                        .ok_or_else(|| format!("line {n}: team {val:?} is not a number"))?
                }
                "name" => p.name = Some(val.to_string()),
                other => return Err(format!("line {n}: unknown player field {other:?}")),
            }
        }
        if p.machine == usize::MAX {
            return Err(format!("line {n}: the player needs machine=<index>"));
        }
        Ok(p)
    }

    pub(crate) fn check(&self) -> Result<(), String> {
        let m = self.machines.len();
        if m == 0 || m > PEER_SLOTS {
            return Err(format!("{m} machines; 1 to {PEER_SLOTS} are allowed"));
        }
        for (i, &id) in self.machines.iter().enumerate() {
            if id == 0 || id == BROADCAST {
                return Err(format!(
                    "machine {i} has the id {id:#x}, which isn't allowed"
                ));
            }
            if self.machines[..i].contains(&id) {
                return Err(format!(
                    "machine {i} has the same id as an earlier one ({id:#x})"
                ));
            }
        }
        if self.host >= m {
            return Err(format!("host = {} but there are {m} machines", self.host));
        }
        if let Some(me) = self.me {
            if me >= m {
                return Err(format!("me = {me} but there are {m} machines"));
            }
        }
        let p = self.players.len();
        if p == 0 || p > PLAYER_SLOTS {
            return Err(format!("{p} players; 1 to {PLAYER_SLOTS} are allowed"));
        }
        for (j, pl) in self.players.iter().enumerate() {
            if pl.xuid == 0 {
                return Err(format!("player {j} has XUID 0"));
            }
            if self.players[..j].iter().any(|q| q.xuid == pl.xuid) {
                return Err(format!(
                    "player {j} has the same XUID as an earlier one ({:#x})",
                    pl.xuid
                ));
            }
            if pl.machine >= m {
                return Err(format!(
                    "player {j} is on machine {} but there are {m}",
                    pl.machine
                ));
            }
            if !(-1..16).contains(&pl.team) {
                return Err(format!(
                    "player {j} has team {}; -1 to 15 are allowed",
                    pl.team
                ));
            }
            if let Some(name) = &pl.name {
                let len = name.chars().count();
                if len == 0 || len > 15 {
                    return Err(format!(
                        "player {j}'s name {name:?} should be 1 to 15 characters"
                    ));
                }
            }
        }
        for i in 0..m {
            if self.on_machine(i).count() > 4 {
                return Err(format!("machine {i} has more than 4 players"));
            }
        }
        if let Some(k) = &self.key {
            if crate::util::parse_hex_bytes(k).is_none_or(|b| b.len() != 16) {
                return Err("key should be 32 hex digits".into());
            }
        }
        Ok(())
    }

    /// The players on machine `m`, in file order, with their indexes.
    pub fn on_machine(&self, m: usize) -> impl Iterator<Item = (usize, &Player)> {
        self.players
            .iter()
            .enumerate()
            .filter(move |(_, p)| p.machine == m)
    }

    /// The machine this launcher is: `--me` if given, else the file's
    /// `me`; an error when neither says or the machine has no player.
    pub fn resolve_me(&self, cli: Option<usize>) -> Result<usize, String> {
        let me = cli
            .or(self.me)
            .ok_or("which machine is this? pass --me <index> (or add me = <index>)")?;
        if me >= self.machines.len() {
            return Err(format!(
                "--me {me}, but the session has {} machines",
                self.machines.len()
            ));
        }
        if self.on_machine(me).next().is_none() {
            return Err(format!("machine {me} has no player in the session"));
        }
        Ok(me)
    }

    /// The local player (the first on machine `me`).
    pub fn local_player(&self, me: usize) -> Option<&Player> {
        self.on_machine(me).map(|(_, p)| p).next()
    }

    pub fn is_host(&self, me: usize) -> bool {
        me == self.host
    }

    /// The relay's address (the first the name resolves to).
    pub fn relay_addr(&self) -> Result<SocketAddr, String> {
        let with_port = if self.relay.parse::<SocketAddr>().is_ok() || self.relay.contains(':') {
            self.relay.clone()
        } else {
            format!("{}:47050", self.relay)
        };
        with_port
            .to_socket_addrs()
            .map_err(|e| format!("relay {:?}: {e}", self.relay))?
            .next()
            .ok_or_else(|| format!("relay {:?} resolves to nothing", self.relay))
    }

    /// Writes the session into the options, after the variant copy and the
    /// offline match fields and before `--set-option`: flags (bits 3 and 6
    /// on the host, both clear elsewhere), the threshold, the host's secure
    /// address and id, the machine list, every player, and this machine's
    /// id.
    ///
    /// Bit 3 makes the engine create a session it hosts as it starts; with
    /// it clear, a networked launch starts a system-link search for a game
    /// to join instead (static read of 1.3528 and a test on the owner's PC,
    /// 2026-10-10: with bit 3 on both, each launcher hosted its own empty
    /// game). What bit 6 does in Halo 2 is not known; it stays on the host
    /// as in Reach.
    pub fn apply(&self, opts: &mut GameOptions, me: usize) {
        let host_bits = options::flags::MULTIPLAYER | options::flags::LISTEN_SERVER;
        let mut flags = opts.get_u16(off::FLAGS);
        if self.is_host(me) {
            flags |= host_bits;
        } else {
            flags &= !host_bits;
        }
        opts.put_u16(off::FLAGS, flags);
        if let Some(t) = self.threshold {
            opts.put_u8(off::UN_0, t);
        }
        opts.put_u64(off::HOST_SECURE_ADDRESS, self.secure);
        // This machine's own id, not the host's: a guest that claims the
        // host's id is turned away as the host's own machine.
        opts.put_u64(off::LOCAL_ADDRESS, self.machines[me]);
        // The whole player block starts clean: the offline fields written
        // before (peer 0 and player 0 at address 123) must not linger.
        let block = off::PLAYER_OPTIONS..off::PLAYER_OPTIONS + options::PLAYER_OPTIONS_SIZE;
        opts.bytes_mut()[block].fill(0);
        for (i, &id) in self.machines.iter().enumerate() {
            opts.put_peer(i, id);
        }
        opts.put_i32(off::PEER_COUNT, self.machines.len() as i32);
        opts.put_i32(off::PLAYER_COUNT, self.players.len() as i32);
        for (j, p) in self.players.iter().enumerate() {
            let at = off::PLAYERS + PLAYER_STRIDE * j;
            let local_count = self.on_machine(p.machine).count() as i32;
            let controller = self
                .on_machine(p.machine)
                .position(|(k, _)| k == j)
                .unwrap_or(0) as i32;
            opts.put_u64(at + player::XUID, p.xuid);
            opts.put_u64(at + player::ADDRESS, self.machines[p.machine]);
            opts.put_i32(at + player::TEAM, p.team);
            opts.put_i32(at + player::LOCAL_COUNT, local_count);
            opts.put_i32(at + player::PEER_INDEX, p.machine as i32);
            opts.put_i32(at + player::CONTROLLER, controller);
        }
        opts.put_u64(off::LOCAL_NETWORK_ID, self.machines[me]);
    }

    /// One line for the log.
    pub fn describe(&self, me: usize) -> String {
        let players: Vec<String> = self
            .players
            .iter()
            .map(|p| {
                format!(
                    "{:#018x}@{}{}{}",
                    p.xuid,
                    p.machine,
                    if p.team >= 0 {
                        format!(" team {}", p.team)
                    } else {
                        String::new()
                    },
                    p.name
                        .as_deref()
                        .map(|n| format!(" {n:?}"))
                        .unwrap_or_default()
                )
            })
            .collect();
        let machines: Vec<String> = self.machines.iter().map(|m| format!("{m:#018x}")).collect();
        format!(
            "session: relay {} room {:#x}, machine {me} of [{}] ({}), host {}, players [{}]{}",
            self.relay,
            self.room,
            machines.join(", "),
            if self.is_host(me) {
                "hosting"
            } else {
                "joining"
            },
            self.host,
            players.join(", "),
            if self.key.is_some() {
                ", member key given"
            } else {
                ""
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO: &str = "\
# two launchers on one PC
relay = 127.0.0.1:47051
room = 0x5EED000000000001
secure = 0x1234567890ABCDEF
host = 0
machine = 0x4832000000000001
machine = 0x4832000000000002   # the guest
player = 0x0009000000000001 machine=0 team=0 name=Host
player = 0x0009000000000002 machine=1 team=1 name=Guest
threshold = 2
";

    #[test]
    fn parses_two_machines() {
        let s = Session::parse(TWO).unwrap();
        assert_eq!(s.relay, "127.0.0.1:47051");
        assert_eq!(s.room, 0x5EED_0000_0000_0001);
        assert_eq!(s.secure, 0x1234_5678_90AB_CDEF);
        assert_eq!(
            s.machines,
            vec![0x4832_0000_0000_0001, 0x4832_0000_0000_0002]
        );
        assert_eq!(s.players.len(), 2);
        assert_eq!(s.players[1].machine, 1);
        assert_eq!(s.players[1].team, 1);
        assert_eq!(s.players[1].name.as_deref(), Some("Guest"));
        assert_eq!(s.threshold, Some(2));
        assert_eq!(s.relay_addr().unwrap(), "127.0.0.1:47051".parse().unwrap());
        assert!(s.resolve_me(None).is_err());
        assert_eq!(s.resolve_me(Some(1)), Ok(1));
        assert!(s.resolve_me(Some(2)).is_err());
        assert_eq!(s.local_player(1).unwrap().xuid, 0x0009_0000_0000_0002);
    }

    #[test]
    fn relay_without_port_uses_47050() {
        let s = Session::parse(&TWO.replace("127.0.0.1:47051", "127.0.0.1")).unwrap();
        assert_eq!(s.relay_addr().unwrap().port(), 47050);
    }

    #[test]
    fn writes_the_roster_like_the_stage_3_runs() {
        let s = Session::parse(TWO).unwrap();
        for me in 0..2 {
            let mut o = GameOptions::new();
            o.apply_base(&options::Launch::offline(44, 0x0009_0000_0000_0001));
            o.apply_match(&options::Launch::offline(44, 0x0009_0000_0000_0001));
            s.apply(&mut o, me);
            let flags = o.get_u16(off::FLAGS);
            assert_eq!(flags, if me == 0 { 0x48 } else { 0x00 }, "machine {me}");
            assert_eq!(o.bytes()[off::UN_0], 2);
            assert_eq!(o.get_u64(0x50), 0x1234_5678_90AB_CDEF);
            assert_eq!(o.get_u64(0x58), s.machines[me], "this machine's own id");
            assert_eq!(o.get_u64(0x60), 0x4832_0000_0000_0001);
            assert_eq!(o.get_u64(0x68), 0x4832_0000_0000_0002);
            assert_eq!(o.get_u64(0x70), 0, "no third machine");
            assert_eq!(o.get_i32(0xE8), 2);
            assert_eq!(o.get_i32(0x2F0), 2);
            // Player 0, the host's.
            assert_eq!(o.get_u64(0xF0), 0x0009_0000_0000_0001);
            assert_eq!(o.get_u64(0xF8), 0x4832_0000_0000_0001);
            assert_eq!(o.get_i32(0x100), 0);
            assert_eq!(o.get_i32(0x104), 1);
            assert_eq!(o.get_i32(0x108), 0);
            assert_eq!(o.get_i32(0x10C), 0);
            // Player 1, the guest's.
            assert_eq!(o.get_u64(0x110), 0x0009_0000_0000_0002);
            assert_eq!(o.get_u64(0x118), 0x4832_0000_0000_0002);
            assert_eq!(o.get_i32(0x120), 1);
            assert_eq!(o.get_i32(0x124), 1);
            assert_eq!(o.get_i32(0x128), 1);
            assert_eq!(o.get_i32(0x12C), 0);
            assert_eq!(o.get_u64(0x2F8), s.machines[me]);
            // The map stays.
            assert_eq!(o.get_i32(off::LEGACY_MAP_ID), 44);
        }
    }

    #[test]
    fn two_players_on_one_machine_get_controllers_0_and_1() {
        let text = TWO.replace(
            "player = 0x0009000000000002 machine=1 team=1 name=Guest",
            "player = 0x0009000000000002 machine=1\nplayer = 0x0009000000000003 machine=1 team=-1",
        );
        let s = Session::parse(&text).unwrap();
        let mut o = GameOptions::new();
        s.apply(&mut o, 1);
        assert_eq!(o.get_i32(0xE8), 3);
        for (j, (count, peer, ctl)) in [(1, 0, 0), (2, 1, 0), (2, 1, 1)].into_iter().enumerate() {
            let at = 0xF0 + 0x20 * j;
            assert_eq!(o.get_i32(at + 0x14), count, "player {j} local count");
            assert_eq!(o.get_i32(at + 0x18), peer, "player {j} machine");
            assert_eq!(o.get_i32(at + 0x1C), ctl, "player {j} controller");
        }
        assert_eq!(o.get_i32(0xF0 + 0x40 + 0x10), -1);
    }

    #[test]
    fn me_from_the_file() {
        let s = Session::parse(&format!("{TWO}me = 1\n")).unwrap();
        assert_eq!(s.resolve_me(None), Ok(1));
        assert_eq!(s.resolve_me(Some(0)), Ok(0), "--me wins");
    }

    #[test]
    fn bad_files() {
        let cases: &[(&str, &str)] = &[
            ("relay = 127.0.0.1:47051\n", "\n"),
            ("room = 0x5EED000000000001\n", "\n"),
            ("host = 0\n", "host = 2\n"),
            (
                "machine = 0x4832000000000002",
                "machine = 0x4832000000000001",
            ),
            ("machine = 0x4832000000000002", "machine = 0"),
            (
                "machine = 0x4832000000000002",
                "machine = 0xFFFFFFFFFFFFFFFF",
            ),
            ("machine=1 team=1", "machine=2 team=1"),
            ("machine=1 team=1", "team=1"),
            ("machine=1 team=1", "machine=1 team=16"),
            ("machine=1 team=1", "machine=1 colour=red"),
            ("name=Guest", "name=AVeryLongGamertag"),
            (
                "0x0009000000000002 machine=1",
                "0x0009000000000001 machine=1",
            ),
            ("0x0009000000000002 machine=1", "0 machine=1"),
            ("threshold = 2", "threshold = 300"),
            ("threshold = 2", "wibble = 2"),
            ("threshold = 2", "no equals sign"),
        ];
        for (from, to) in cases {
            let text = TWO.replace(from, to);
            assert_ne!(text, TWO, "case {from:?} didn't change the text");
            assert!(
                Session::parse(&text).is_err(),
                "{from:?} -> {to:?} should fail"
            );
        }
        assert!(Session::parse(&format!("{TWO}key = 00ff\n")).is_err());
        assert!(Session::parse(&format!("{TWO}key = 000102030405060708090a0b0c0d0e0f\n")).is_ok());
        assert!(Session::parse(&format!("{TWO}me = 2\n")).is_err());
        let nobody_on_1 = TWO.replace(
            "player = 0x0009000000000002 machine=1 team=1 name=Guest\n",
            "",
        );
        let s = Session::parse(&nobody_on_1).unwrap();
        assert!(s.resolve_me(Some(1)).is_err());
    }

    #[test]
    fn describe_says_who_hosts() {
        let s = Session::parse(TWO).unwrap();
        assert!(s.describe(0).contains("hosting"));
        assert!(s.describe(1).contains("joining"));
        assert!(s.describe(1).contains("\"Guest\""));
    }
}
