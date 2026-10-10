//! Online play in the game, like Halo 2 on Xbox Live: signing in to the
//! online service (h2live), the party lobby, who else is online, and
//! searching the matchmaking playlists. The menus show what the service
//! last said through an `OnlineView`, and what players choose there comes
//! back as menu actions, which become requests to the service.
//!
//! H2_LIVE names the service: ws://host:port or wss://host for one on the
//! network (reached over a WebSocket, dialed in the background), or `mem`
//! to run one inside the game, for testing, with H2_LIVE_FAKE_PLAYERS=<n>
//! made-up players signed in to it: some in parties, one inviting us into
//! theirs, all taking our invites.

use crate::menu::{map_title, Action, MapChoice, Screen, Sound};
use crate::{App, Mode};
use blam_cache::{text, GroupTag, MapSet};
use h2live::client::{self, LiveClient, LiveEvent, RelayLeg, View};
use h2live::server::{Route, Server};
use h2net::live::{
    self, Activity, LinkInfo, MatchInfo, OnlinePlayer, PartyInfo, PartyMember, PlaylistInfo,
    Privacy, Stage, ToServer, QUICKMATCH,
};
use h2net::Connection;
use h2sim::bot::{bot_look, bot_name};
use std::collections::HashMap;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

mod custom;
mod matches;
pub mod recent;

use custom::Custom;
use matches::Matched;
use recent::{Recent, RecentPlayers};

/// The online server everyone signs in to, unless H2_LIVE or a `server=`
/// line in the profile names another. None yet: until there is, it's the
/// one on this PC.
const DEFAULT_LIVE_SERVER: &str = "";
/// h2live on this PC, which is tried first so the PC running it finds it:
/// where it listens, and how long it has to answer.
const HERE: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 47050);
const HERE_WAIT: Duration = Duration::from_millis(100);
/// Longest signing in waits for the service to answer: one hosted for free
/// sleeps while no one plays, and takes up to a minute to wake.
const DIAL_WAIT: Duration = Duration::from_secs(90);
/// Seconds of waiting after which the service is probably waking up.
const WAKING: f64 = 5.0;
/// Longest a relay leg's connection may take.
const LEG_WAIT: Duration = Duration::from_secs(20);
/// For testing (H2_LIVE_AUTO): seconds the party stays in its lobby before
/// it searches again by itself.
const AUTO_WAIT: f64 = 10.0;
/// For testing: seconds after it began before signing in that failed is
/// tried again (the service turns away more than a few sign-ins a minute
/// from one address).
const AUTO_RETRY: f64 = 15.0;
/// Shown when the service turned down our gamertag.
const TAKEN: &str = "THAT GAMERTAG IS TAKEN. TYPE ANOTHER, THEN SIGN IN";
/// Most made-up players an in-game service has.
const MAX_FAKES: usize = 15;
/// Seconds a search we asked for may take to start before we give up on
/// it (the service always answers sooner).
const ASK_WAIT: f64 = 10.0;

/// Halo 2's string lists the online screens use, in mainmenu.map.
const PLAYLIST_TEXT: &str = "multiplayer\\matchmaking_hopper_descriptions";
const PROGRESS_TEXT: &str =
    "ui\\screens\\game_shell\\xbox_live\\matchmaking_progress_dialog\\strings";
const PLAYER_TEXT: &str = "ui\\screens\\game_shell\\online_y_menu\\online_y_menu";
const LOBBY_TEXT: &str = "ui\\screens\\game_shell\\pregame_lobby\\pregame_lobby";

/// Halo 2's own words for the online screens, from mainmenu.map's string
/// lists, by their names there. Without mainmenu.map the screens use
/// plainer words of their own.
#[derive(Default)]
pub struct LiveText {
    /// Playlists' names and descriptions (`<key>_title`, `<key>_description`).
    playlists: HashMap<String, String>,
    /// How a search is going.
    progress: HashMap<String, String>,
    /// What other players are doing.
    players: HashMap<String, String>,
    /// The party lobby's messages.
    lobby: HashMap<String, String>,
}

impl LiveText {
    /// The words from the mainmenu.map in the maps folder `dir`.
    pub fn load(dir: &Path) -> LiveText {
        let Ok(mut set) = MapSet::open(dir.join("mainmenu.map")) else {
            return LiveText::default();
        };
        let Ok(table) = text::language_table(&mut set) else {
            return LiveText::default();
        };
        let mut list = |name| strings(&mut set, &table, name);
        LiveText {
            playlists: list(PLAYLIST_TEXT),
            progress: list(PROGRESS_TEXT),
            players: list(PLAYER_TEXT),
            lobby: list(LOBBY_TEXT),
        }
    }

    /// A playlist's name.
    pub fn playlist_name(&self, p: &PlaylistInfo) -> String {
        words(&self.playlists, &format!("{}_title", p.key), &p.name)
    }

    /// What a playlist plays, when Halo 2 says.
    pub fn playlist_description(&self, p: &PlaylistInfo) -> String {
        words(&self.playlists, &format!("{}_description", p.key), "")
    }
}

/// One string list's strings, by name.
fn strings(set: &mut MapSet, table: &[(u32, String)], name: &str) -> HashMap<String, String> {
    let tag = GroupTag::parse("unic").and_then(|g| set.map.find_tag(g, name));
    let Some(datum) = tag.map(|t| t.datum) else {
        println!("warning: no {name} in mainmenu.map");
        return HashMap::new();
    };
    let list = text::unicode_strings(set, table, datum).unwrap_or_default();
    list.into_iter()
        .filter_map(|(id, s)| Some((set.map.string_id(id)?.to_string(), s)))
        .collect()
}

/// One of a list's strings, in capitals like all the menus' text, or
/// `otherwise`.
fn words(list: &HashMap<String, String>, name: &str, otherwise: &str) -> String {
    list.get(name)
        .map_or(otherwise, String::as_str)
        .to_uppercase()
}

/// `words` with numbers filled in (see `fill`).
fn words_with(
    list: &HashMap<String, String>,
    name: &str,
    otherwise: &str,
    numbers: &[u32],
) -> String {
    let format = list.get(name).map_or(otherwise, String::as_str);
    fill(format, numbers).to_uppercase()
}

/// Halo 2's text with its numbers (`%d`, `%02d`...) filled in, in order.
pub fn fill(format: &str, numbers: &[u32]) -> String {
    let mut out = String::new();
    let mut numbers = numbers.iter().copied();
    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let mut width = String::new();
        while let Some(d) = chars.next_if(char::is_ascii_digit) {
            width.push(d);
        }
        match chars.next() {
            Some('d') => {
                let n = numbers.next().unwrap_or(0);
                let w = width.parse().unwrap_or(0);
                out += &if width.starts_with('0') {
                    format!("{n:0w$}")
                } else {
                    format!("{n:w$}")
                };
            }
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out += &width;
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

/// "1:05".
pub fn clock_text(seconds: f64) -> String {
    let s = seconds.max(0.0) as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

/// What the online screens show.
pub struct OnlineView<'a> {
    /// What the service last said, once signed in.
    pub live: Option<&'a View>,
    /// Why signing in failed or the link was lost (otherwise, until signed
    /// in, still signing in).
    pub failed: Option<&'a str>,
    /// The service is slow to answer: waking up, probably.
    pub waking: bool,
    pub text: &'a LiveText,
    /// The party's search: its playlist (or `QUICKMATCH`) and seconds so
    /// far.
    pub search: Option<(u8, f64)>,
    /// The match we're in, from MATCH until we're back in the party lobby.
    pub game: Option<&'a MatchInfo>,
    /// Before its game: seconds left to the pregame countdown, until the
    /// service says to start.
    pub countdown: Option<f64>,
    /// After its game: seconds until we're back in the party lobby.
    pub returning: Option<f64>,
    /// In the party's custom game.
    pub custom: bool,
    /// The maps this PC has, by file name.
    pub maps: &'a [String],
    /// Who we played with lately, newest first.
    pub recent: &'a [Recent],
    /// Now, in Unix time (to say how long ago that was).
    pub time: u64,
}

impl OnlineView<'_> {
    /// Our account.
    pub fn me(&self) -> u64 {
        let welcome = self.live.and_then(|v| v.welcome.as_ref());
        welcome.map_or(0, |w| w.account)
    }

    pub fn party(&self) -> Option<&PartyInfo> {
        self.live?.party.as_ref()
    }

    /// We lead our party, so choose what it plays.
    pub fn leads(&self) -> bool {
        self.party().is_some_and(|p| p.leader == self.me())
    }

    /// The party has people other than us.
    pub fn partied(&self) -> bool {
        self.party().is_some_and(|p| p.members.len() > 1)
    }

    /// The party's leader's gamertag.
    pub fn leader(&self) -> Option<&str> {
        let party = self.party()?;
        let leader = party.members.iter().find(|m| m.account == party.leader)?;
        Some(&leader.gamertag)
    }

    /// Everyone else online.
    pub fn others(&self) -> Vec<&OnlinePlayer> {
        let me = self.me();
        let online = self.live.map_or(&[][..], |v| &v.online);
        online.iter().filter(|p| p.account != me).collect()
    }

    pub fn player(&self, account: u64) -> Option<&OnlinePlayer> {
        self.others().into_iter().find(|p| p.account == account)
    }

    /// Someone we played with lately, by their account.
    pub fn recent_player(&self, account: u64) -> Option<&Recent> {
        self.recent.iter().find(|r| r.account == account)
    }

    /// What someone is doing if they're online (see `doing`), or else
    /// that they're offline.
    pub fn status(&self, account: u64) -> String {
        match self.player(account) {
            Some(p) => self.doing(p),
            None => words(&self.text.players, "offline", "OFFLINE"),
        }
    }

    /// What we last played with a recent player, where and when: "TEAM
    /// SNIPERS ON LOCKOUT, 5 MINUTES AGO".
    pub fn last_played(&self, r: &Recent) -> String {
        let what = match self.playlists().iter().find(|p| p.key == r.played) {
            Some(p) => self.text.playlist_name(p),
            None if r.played == recent::CUSTOM => "CUSTOM GAME".into(),
            None => r.played.replace('_', " ").to_uppercase(),
        };
        let when = recent::ago(r.when, self.time);
        format!("{what} ON {}, {when}", map_title(&r.map))
    }

    pub fn playlists(&self) -> &[PlaylistInfo] {
        self.live.map_or(&[][..], |v| &v.playlists)
    }

    /// The latest invite: the party, and who asked us.
    pub fn invite(&self) -> Option<&(u64, String)> {
        self.live?.invites.last()
    }

    /// A player's party is ours.
    pub fn with_us(&self, p: &OnlinePlayer) -> bool {
        self.party().is_some_and(|party| party.id == p.party)
    }

    /// What a player is doing, as Halo 2's player list said it.
    pub fn doing(&self, p: &OnlinePlayer) -> String {
        let t = &self.text.players;
        if self.with_us(p) && self.party().is_some_and(|party| party.leader == p.account) {
            return words(t, "online_your_party_leader", "YOUR PARTY LEADER");
        }
        if self.with_us(p) {
            return words(t, "online_in_your_party", "IN YOUR PARTY");
        }
        let (members, openings) = (u32::from(p.size), u32::from(p.openings));
        let (name, otherwise, n) = match (p.activity, p.open) {
            (Activity::Lobby, _) if p.size <= 1 => return words(t, "online_generic", "ONLINE"),
            (Activity::Lobby, true) => {
                ("in_open_pregame_lobby", "IN A PARTY, %d OPENINGS", openings)
            }
            (Activity::Lobby, false) => ("in_closed_pregame_lobby", "IN A PARTY OF %d", members),
            (Activity::Searching, _) => ("searching_for_games", "SEARCHING, PARTY OF %d", members),
            (Activity::Playing, _) => {
                ("playing_matchmade_game", "IN A MATCH, PARTY OF %d", members)
            }
            (Activity::Custom, true) => (
                "in_open_arranged_game",
                "IN A CUSTOM GAME, %d OPENINGS",
                openings,
            ),
            (Activity::Custom, false) => {
                ("in_closed_arranged_game", "IN A CUSTOM GAME OF %d", members)
            }
        };
        words_with(t, name, otherwise, &[n])
    }

    /// The party's size, splitscreen guests too.
    pub fn party_size(&self) -> usize {
        let members = self.party().map_or(&[][..], |p| &p.members);
        members.iter().map(|m| 1 + usize::from(m.guests)).sum()
    }

    /// The playlist's maps someone in the party doesn't have (or not the
    /// same copy of as the others). Halo 2 marked the playlists of a map
    /// pack a player didn't have, and they couldn't search them.
    pub fn missing_maps<'p>(&self, p: &'p PlaylistInfo) -> Vec<&'p str> {
        let Some(party) = self.party() else {
            return Vec::new();
        };
        let shared = |map: &&str| party.maps.iter().any(|m| m.eq_ignore_ascii_case(map));
        p.maps
            .iter()
            .map(String::as_str)
            .filter(|m| !shared(m))
            .collect()
    }

    /// Who's missing which of a playlist's maps, if anyone is: "YOU'RE
    /// MISSING MAPS: WARLOCK, GEMINI", or someone else in the party is.
    pub fn missing_text(&self, p: &PlaylistInfo) -> Option<String> {
        let missing = self.missing_maps(p);
        let ours = |map: &&str| self.maps.iter().any(|m| m.eq_ignore_ascii_case(map));
        let mine: Vec<&str> = missing.iter().copied().filter(|m| !ours(m)).collect();
        let (who, maps) = if mine.is_empty() {
            ("SOMEONE IN YOUR PARTY IS", missing)
        } else {
            ("YOU'RE", mine)
        };
        if maps.is_empty() {
            return None;
        }
        let titles: Vec<String> = maps.iter().take(3).map(|m| map_title(m)).collect();
        let mut names = titles.join(", ");
        if maps.len() > 3 {
            names += &format!(" AND {} MORE", maps.len() - 3);
        }
        Some(format!("{who} MISSING MAPS: {names}"))
    }

    /// Why the party can't search a playlist, if it can't.
    pub fn cant_search(&self, p: &PlaylistInfo) -> Option<String> {
        if let Some(missing) = self.missing_text(p) {
            return Some(missing);
        }
        let t = &self.text.lobby;
        let members = self.party().map_or(&[][..], |p| &p.members);
        if !p.guests && members.iter().any(|m| m.guests > 0) {
            let otherwise = "SPLITSCREEN GUESTS CAN'T PLAY THIS PLAYLIST";
            return Some(words(t, "squad_can_not_contain_guests", otherwise));
        }
        if self.party_size() > usize::from(p.party_max) {
            let otherwise = "YOUR PARTY IS TOO BIG FOR THIS PLAYLIST";
            return Some(words(t, "squad_too_large", otherwise));
        }
        None
    }

    /// The name of what the party searches.
    pub fn search_name(&self) -> String {
        self.playlist_name(self.search.map_or(QUICKMATCH, |(p, _)| p))
    }

    /// A playlist's name, by its id.
    pub fn playlist_name(&self, id: u8) -> String {
        match self.playlists().iter().find(|p| p.id == id) {
            Some(p) => self.text.playlist_name(p),
            None => "QUICKMATCH".into(),
        }
    }

    /// How the match's pregame is going: "GAME ABOUT TO START!" with the
    /// countdown, then waiting for everyone's map once the service said to
    /// start.
    pub fn pregame_status(&self) -> (String, Option<String>) {
        match self.countdown {
            Some(left) => ("GAME ABOUT TO START!".into(), Some(clock_text(left.ceil()))),
            None => {
                let otherwise = "WAITING FOR EVERYONE TO LOAD THE MAP...";
                (words(&self.text.lobby, "precaching", otherwise), None)
            }
        }
    }

    /// How the party's search is going, in Halo 2's words: a headline, and
    /// maybe a line more.
    pub fn search_status(&self) -> (String, Option<String>) {
        let t = &self.text.progress;
        let searching = || words(t, "searching_for_games", "SEARCHING FOR A GAME...");
        let gathering = || words(t, "gathering_game", "WAITING FOR MORE PLAYERS...");
        let Some(s) = self.live.and_then(|v| v.status) else {
            return (searching(), None);
        };
        let need = u32::from(s.need);
        match s.stage {
            Stage::Searching if s.low > 0 => {
                let similar = words(t, "callout1", "FINDING PLAYERS OF A SIMILAR LEVEL.");
                (searching(), Some(similar))
            }
            Stage::Searching => (searching(), None),
            Stage::Gathering => {
                let needed = words_with(t, "players_needed", "%d MORE PLAYERS NEEDED", &[need]);
                (gathering(), Some(needed))
            }
            Stage::WaitingToFill => {
                let at = [u32::from(s.seconds) / 60, u32::from(s.seconds) % 60];
                let starting = words_with(t, "waiting_to_fill", "STARTING IN %02d:%02d", &at);
                let players = words(t, "players", "PLAYERS:");
                (starting, Some(format!("{players} {}", s.have)))
            }
            Stage::Balancing => {
                let otherwise = "%d MORE PLAYERS NEEDED FOR EVEN TEAMS";
                let needed = words_with(t, "players_needed_to_balance", otherwise, &[need]);
                (gathering(), Some(needed))
            }
            Stage::Joining => (words(t, "joining_game", "JOINING A GAME..."), None),
            Stage::Starting => (words(t, "starting_game", "STARTING THE GAME..."), None),
            Stage::Failed => (words(t, "matchmaking_failed", "MATCHMAKING FAILED!"), None),
        }
    }

    /// Why the party's last search ended, if it gave up: "MATCHMAKING
    /// FAILED!"
    pub fn search_failed(&self) -> Option<String> {
        let failed = self.live?.status?.stage == Stage::Failed;
        let t = &self.text.progress;
        failed.then(|| words(t, "matchmaking_failed", "MATCHMAKING FAILED!"))
    }

    /// "SARGE IS THE PARTY LEADER": Halo 2's text marks where the name
    /// goes with U+E416.
    pub fn leader_text(&self) -> Option<String> {
        let otherwise = "\u{e416} IS THE PARTY LEADER";
        let text = words(&self.text.lobby, "party_leader", otherwise);
        Some(text.replace('\u{e416}', self.leader()?))
    }
}

/// This PC's link to the online service.
pub struct Online {
    link: Link,
    /// The service's address (H2_LIVE), and when we began to dial it.
    address: String,
    dialed: f64,
    /// The clock the service's messages are timed by.
    clock: Instant,
    /// Where the PC's key is kept (identity.key).
    identity: Option<PathBuf>,
    /// Why we're not signed in, once signing in failed or the link was lost.
    failed: Option<String>,
    /// The party's search: its playlist (or `QUICKMATCH`) and when it began,
    /// while it searches.
    search: Option<(u8, f64)>,
    /// When we asked for a search, until the service starts it or says why
    /// not. (It shows as searching meanwhile.)
    asked: Option<f64>,
    /// What the party was last doing.
    activity: Activity,
    /// The match we're in, until we're back in the party lobby.
    matched: Option<Matched>,
    /// The party's custom game we're in, until we're back in its lobby.
    custom: Option<Custom>,
    /// Relay legs on their way to the other end.
    legs: Vec<Leg>,
    /// A service run in the game, for testing.
    test: Option<TestService>,
    pub text: LiveText,
    /// The maps this PC signed in with, by file name.
    maps: Vec<String>,
    /// Who we played with lately, kept beside our key once signed in.
    recent: RecentPlayers,
    /// For testing (H2_LIVE_AUTO=search:<key>, or H2_LIVE_BOT=<key>): the
    /// playlist the party searches by itself, and when it was last busy.
    auto: Option<String>,
    idle: f64,
}

enum Link {
    Offline,
    /// Waiting for the connection, to sign in as this.
    Dialing(Receiver<Result<Connection, String>>, client::Profile),
    /// Signing in, then signed in.
    Live(Box<LiveClient>),
}

/// A relay leg on its way: its connection to the service being dialed,
/// then waiting there for the other end.
enum Leg {
    Dialing(LinkInfo, Receiver<Result<Connection, String>>),
    Open(RelayLeg),
}

/// Where the PC's key is kept: H2_IDENTITY, or identity.key beside the
/// profile.
pub fn identity_path() -> Option<PathBuf> {
    match std::env::var_os("H2_IDENTITY") {
        Some(p) => Some(PathBuf::from(p)),
        None => crate::profile::beside("identity.key"),
    }
}

/// Where a file of the key at `identity`'s own is kept: as `usual` beside
/// identity.key, and as `<name>-<kind>.txt` beside any other key, so keys
/// that share a folder (H2_IDENTITY's, say) don't share it. Its stat card
/// is live-card.txt (or `<name>-card.txt`), and its recent players
/// recent-players.txt (or `<name>-recent.txt`).
fn kept_with(identity: &Path, usual: &str, kind: &str) -> PathBuf {
    if identity.file_name() == Some("identity.key".as_ref()) {
        return identity.with_file_name(usual);
    }
    let name = identity.file_stem().unwrap_or_default().to_string_lossy();
    identity.with_file_name(format!("{name}-{kind}.txt"))
}

/// Where on the service at `address` a path is (`live` for signing in,
/// `link` for relay legs), whatever path the address had.
fn service_url(address: &str, path: &str) -> String {
    let base = address.trim_end_matches('/');
    let base = ["/live", "/link"]
        .iter()
        .find_map(|p| base.strip_suffix(p))
        .unwrap_or(base);
    format!("{base}/{path}")
}

/// The maps this PC has, as the service names them.
fn map_hashes(maps: &[MapChoice]) -> Vec<(String, u64)> {
    let hash = |m: &MapChoice| Some((m.name.clone(), live::map_hash(&m.path).ok()?));
    maps.iter().filter_map(hash).collect()
}

/// The online server to sign in to when H2_LIVE doesn't say: h2live on
/// this PC if it's `here`, or else the one `chosen` (the profile's
/// `server=`), or else the usual one. Web addresses, as a browser shows
/// them, become WebSocket ones.
fn live_server(chosen: &str, here: impl FnOnce() -> bool) -> String {
    let elsewhere = [chosen.trim(), DEFAULT_LIVE_SERVER]
        .into_iter()
        .find(|a| !a.is_empty());
    let Some(address) = elsewhere.filter(|_| !here()) else {
        return format!("ws://{HERE}");
    };
    let address = address.trim_end_matches('/');
    let lower = address.to_ascii_lowercase();
    if lower.starts_with("https://") {
        format!("wss://{}", &address["https://".len()..])
    } else if lower.starts_with("http://") {
        format!("ws://{}", &address["http://".len()..])
    } else {
        address.to_string()
    }
}

/// Whether something listens at `address`, taking calls within `wait`.
/// It's asked what a host's health check asks, so a server there has
/// nothing to complain about.
fn listens(address: SocketAddr, wait: Duration) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&address, wait) else {
        return false;
    };
    let _ = stream.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n");
    true
}

impl Online {
    pub fn new(text: LiveText, identity: Option<PathBuf>) -> Online {
        Online {
            link: Link::Offline,
            address: String::new(),
            dialed: 0.0,
            clock: Instant::now(),
            identity,
            failed: None,
            search: None,
            asked: None,
            activity: Activity::Lobby,
            matched: None,
            custom: None,
            legs: Vec::new(),
            test: None,
            text,
            maps: Vec::new(),
            recent: RecentPlayers::default(),
            auto: std::env::var("H2_LIVE_AUTO")
                .ok()
                .and_then(|v| Some(v.strip_prefix("search:")?.to_string()))
                .or_else(|| std::env::var("H2_LIVE_BOT").ok()),
            idle: 0.0,
        }
    }

    /// Seconds on the service's clock.
    pub fn now(&self) -> f64 {
        self.clock.elapsed().as_secs_f64()
    }

    /// What the online screens show; none when we never went online or
    /// signed out.
    pub fn view(&self, now: f64) -> Option<OnlineView<'_>> {
        let live = match &self.link {
            Link::Offline if self.failed.is_none() => return None,
            Link::Live(client) if client.signed_in() => Some(&client.view),
            _ => None,
        };
        let m = self.matched.as_ref();
        Some(OnlineView {
            live,
            failed: self.failed.as_deref(),
            waking: matches!(self.link, Link::Dialing(..)) && now - self.dialed > WAKING,
            text: &self.text,
            search: self.search.map(|(p, since)| (p, now - since)),
            game: m.map(|m| &m.info),
            countdown: m.and_then(|m| m.countdown(now)),
            returning: m.and_then(|m| m.returning(now)),
            custom: self.custom.is_some(),
            maps: &self.maps,
            recent: &self.recent.list,
            time: recent::unix_now(),
        })
    }

    /// Start signing in to the service at `address` as `profile`, with
    /// `fakes` made-up players if the service runs in the game.
    pub fn connect(&mut self, address: &str, profile: client::Profile, fakes: usize, now: f64) {
        self.sign_out();
        self.address = address.to_string();
        self.dialed = now;
        self.maps = profile.maps.iter().map(|(name, _)| name.clone()).collect();
        match self.dial(address, fakes, now) {
            Ok(connection) => self.link = Link::Dialing(connection, profile),
            Err(why) => self.failed = Some(why),
        }
    }

    /// The connection to the service at `address`, on its way: `mem` for
    /// one run in the game, ws://host:port or wss://host for one on the
    /// network.
    fn dial(
        &mut self,
        address: &str,
        fakes: usize,
        now: f64,
    ) -> Result<Receiver<Result<Connection, String>>, String> {
        let (tx, rx) = mpsc::channel();
        if address == "mem" {
            let mut test = TestService::start(fakes, now)?;
            let ours = test.connect(Route::Live, IpAddr::V4(Ipv4Addr::LOCALHOST), now);
            self.test = Some(test);
            let _ = tx.send(Ok(ours));
            return Ok(rx);
        }
        let lower = address.to_ascii_lowercase();
        if lower.starts_with("ws://") || lower.starts_with("wss://") {
            return Ok(h2net::dial(&service_url(address, "live"), DIAL_WAIT));
        }
        Err(format!("NO ONLINE SERVICE AT {}", address.to_uppercase()))
    }

    /// Leave the service, and our party.
    pub fn sign_out(&mut self) {
        self.link = Link::Offline;
        self.failed = None;
        self.search = None;
        self.asked = None;
        self.activity = Activity::Lobby;
        self.matched = None;
        self.custom = None;
        self.legs.clear();
        self.test = None;
    }

    /// Signed in, and still connected.
    pub fn signed_in(&self) -> bool {
        matches!(&self.link, Link::Live(client) if client.signed_in())
    }

    /// Our account and gamertag, once signed in.
    fn me(&self) -> Option<(u64, &str)> {
        let Link::Live(client) = &self.link else {
            return None;
        };
        let welcome = client.view.welcome.as_ref()?;
        Some((welcome.account, &welcome.gamertag))
    }

    /// Tell the service something.
    fn send(&mut self, message: ToServer) {
        if let Link::Live(client) = &mut self.link {
            client.send(message);
        }
    }

    /// We just played with `players` (their account, gamertag and level),
    /// in the playlist `played` (by key) or a custom game, on `map`: they're
    /// our most recent players.
    fn met<'p>(
        &mut self,
        players: impl Iterator<Item = (u64, &'p str, u8)>,
        played: &str,
        map: &str,
    ) {
        let me = self.me().map(|(account, _)| account);
        let when = recent::unix_now();
        let recent = |(account, gamertag, level): (u64, &str, u8)| Recent {
            account,
            gamertag: gamertag.to_string(),
            level,
            played: played.to_string(),
            map: map.to_string(),
            when,
        };
        let others = players.filter(|&(account, ..)| Some(account) != me);
        self.recent.met(others.map(recent).collect());
    }

    /// A playlist's key, by its id ("matchmaking" if the service never
    /// said).
    fn playlist_key(&self, id: u8) -> String {
        let playlists = match &self.link {
            Link::Live(client) => &client.view.playlists[..],
            _ => &[],
        };
        let p = playlists.iter().find(|p| p.id == id);
        p.map_or_else(|| "matchmaking".into(), |p| p.key.clone())
    }

    /// In a match, from MATCH until back in the party lobby.
    pub fn in_match(&self) -> bool {
        self.matched.is_some()
    }

    /// In the party's custom game, from CUSTOM_OPEN until back in the
    /// party lobby.
    pub fn in_custom(&self) -> bool {
        self.custom.is_some()
    }

    /// The party's members, as the service last said.
    fn party_members(&self) -> Vec<PartyMember> {
        let party = match &self.link {
            Link::Live(client) => client.view.party.as_ref(),
            _ => None,
        };
        party.map_or_else(Vec::new, |p| p.members.clone())
    }

    /// In the party's custom game: a member's level (their best), by
    /// gamertag.
    pub fn party_level(&self, gamertag: &str) -> Option<u8> {
        let Link::Live(client) = &self.link else {
            return None;
        };
        let party = client.view.party.as_ref().filter(|_| self.in_custom())?;
        let member = party.members.iter().find(|m| m.gamertag == gamertag)?;
        Some(member.best)
    }

    /// Open a relay leg for `link` (LINK): a new connection to the
    /// service, which joins it to the other end.
    fn open_leg(&mut self, link: LinkInfo, now: f64) {
        let Link::Live(client) = &self.link else {
            return;
        };
        let leg = match &mut self.test {
            Some(test) => {
                let conn = test.connect(Route::Link, IpAddr::V4(Ipv4Addr::LOCALHOST), now);
                Leg::Open(client.open_leg(link, conn))
            }
            None => {
                let url = service_url(&self.address, "link");
                Leg::Dialing(link, h2net::dial(&url, LEG_WAIT))
            }
        };
        self.legs.push(leg);
    }

    /// Relay legs whose other end came: the link each is for, and the
    /// connection to that end. Legs that failed are given up on.
    fn linked(&mut self) -> Vec<(LinkInfo, Connection)> {
        let Link::Live(client) = &self.link else {
            self.legs.clear();
            return Vec::new();
        };
        let mut linked = Vec::new();
        for leg in std::mem::take(&mut self.legs) {
            let leg = match leg {
                Leg::Dialing(link, rx) => match rx.try_recv() {
                    Ok(Ok(conn)) => Leg::Open(client.open_leg(link, conn)),
                    Err(TryRecvError::Empty) => Leg::Dialing(link, rx),
                    Ok(Err(why)) => {
                        println!("live: no relay leg to {}: {why}", link.gamertag);
                        continue;
                    }
                    Err(TryRecvError::Disconnected) => continue,
                },
                open => open,
            };
            match leg {
                Leg::Open(mut relay) => match relay.poll() {
                    Ok(Some(conn)) => linked.push((relay.link, conn)),
                    Ok(None) => self.legs.push(Leg::Open(relay)),
                    Err(why) => println!("live: relay leg to {}: {why}", relay.link.gamertag),
                },
                dialing => self.legs.push(dialing),
            }
        }
        linked
    }

    /// Keep up with the service at time `now`: what it said. Signing in
    /// failing or the link going come as `Refused` or `Lost`, after which
    /// we're signed out.
    pub fn poll(&mut self, now: f64) -> Vec<LiveEvent> {
        if let Some(test) = &mut self.test {
            test.poll(now);
        }
        if let Link::Dialing(rx, profile) = &self.link {
            match rx.try_recv() {
                Ok(Ok(conn)) => {
                    let profile = profile.clone();
                    if let Err(why) = self.sign_in(conn, &profile, now) {
                        return self.lost(why);
                    }
                }
                Ok(Err(why)) => return self.lost(why.to_uppercase()),
                Err(TryRecvError::Empty) => return Vec::new(),
                Err(TryRecvError::Disconnected) => return self.lost("COULDN'T CONNECT".into()),
            }
        }
        let Link::Live(client) = &mut self.link else {
            return Vec::new();
        };
        let events = client.poll(now);
        let searching = client.view.party.as_ref().map(|p| p.activity) == Some(Activity::Searching);
        for e in &events {
            match e {
                LiveEvent::Refused(why) | LiveEvent::Lost(why) => {
                    self.link = Link::Offline;
                    self.failed = Some(why.to_uppercase());
                    self.search = None;
                }
                // The service says why it won't start a search we asked
                // for with a notice.
                LiveEvent::Notice(_) if !searching => self.asked = None,
                _ => {}
            }
        }
        events
    }

    /// Signing in failed before it began.
    fn lost(&mut self, why: String) -> Vec<LiveEvent> {
        self.link = Link::Offline;
        self.failed = Some(why.clone());
        vec![LiveEvent::Lost(why)]
    }

    /// Sign in over `conn` with this PC's key, keeping our stat card and
    /// recent players beside it (or with the service, when it runs in the
    /// game: its cards and players are no good to a real one).
    fn sign_in(
        &mut self,
        conn: Connection,
        profile: &client::Profile,
        now: f64,
    ) -> Result<(), String> {
        let path = self.identity.clone().ok_or("NOWHERE TO KEEP YOUR KEY")?;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            let _ = std::fs::create_dir_all(dir);
        }
        let key = client::identity(&path)
            .map_err(|e| format!("CAN'T READ {}: {e}", path.display()).to_uppercase())?;
        let (card, recent) = match &self.test {
            Some(test) => (
                test.dir.join("live-card.txt"),
                test.dir.join("recent-players.txt"),
            ),
            None => (
                kept_with(&path, "live-card.txt", "card"),
                kept_with(&path, "recent-players.txt", "recent"),
            ),
        };
        self.recent = RecentPlayers::load(&recent);
        let client = LiveClient::new(conn, key, profile, &card, now);
        self.link = Link::Live(Box::new(client));
        Ok(())
    }

    /// Ask the service for what a menu action wants.
    pub fn ask(&mut self, action: &Action, now: f64) {
        let Link::Live(client) = &mut self.link else {
            return;
        };
        let message = match *action {
            Action::Search(playlist) => {
                let playlist = playlist.unwrap_or(QUICKMATCH);
                self.search = Some((playlist, now));
                self.asked = Some(now);
                self.activity = Activity::Searching;
                client.view.status = None;
                ToServer::Search(playlist)
            }
            Action::CancelSearch => {
                self.search = None;
                ToServer::Cancel
            }
            Action::Custom => ToServer::Custom,
            Action::Invite(account) => ToServer::Invite(account),
            Action::JoinParty(party) => ToServer::JoinParty(party),
            Action::Accept(party) => ToServer::Accept(party),
            Action::LeaveParty => ToServer::LeaveParty,
            Action::Kick(account) => ToServer::Kick(account),
            Action::Promote(account) => ToServer::Promote(account),
            Action::Privacy(privacy) => ToServer::Privacy(privacy),
            _ => return,
        };
        client.send(message);
    }

    /// Follow the party into and out of what its leader starts and stops:
    /// searches (one that found a match isn't stopped: the match comes
    /// next), and custom games.
    pub fn follow_party(&mut self, now: f64) -> Option<Followed> {
        let Link::Live(client) = &self.link else {
            return None;
        };
        let party = client.view.party.as_ref();
        let activity = party.map_or(Activity::Lobby, |p| p.activity);
        if let Some(asked) = self.asked {
            if activity == Activity::Lobby && now - asked < ASK_WAIT {
                return None;
            }
            self.asked = None;
        }
        let was = std::mem::replace(&mut self.activity, activity);
        match (was, activity) {
            (Activity::Searching, Activity::Searching) => None,
            (_, Activity::Searching) => {
                let playlist = party.map_or(QUICKMATCH, |p| p.playlist);
                self.search.get_or_insert((playlist, now));
                Some(Followed::Searching)
            }
            (Activity::Searching, Activity::Playing) => {
                self.search = None;
                None
            }
            (Activity::Searching, _) => {
                self.search = None;
                Some(Followed::Stopped)
            }
            (Activity::Custom, Activity::Custom) => None,
            (Activity::Custom, _) => Some(Followed::CustomOver),
            _ => None,
        }
    }
}

/// What the party did, for us to follow (see `Online::follow_party`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Followed {
    /// It started searching.
    Searching,
    /// It stopped (or the service wouldn't start our search).
    Stopped,
    /// Its custom game is over.
    CustomOver,
}

/// The online screens (out of a match).
fn online_screen(screen: Screen) -> bool {
    matches!(
        screen,
        Screen::Live
            | Screen::Players
            | Screen::RecentPlayers
            | Screen::Player
            | Screen::Playlists
            | Screen::Matchmaking
    )
}

impl App {
    /// Sign in to the online service H2_LIVE names, or else `live_server`'s
    /// (ONLINE on the main menu, or SIGN IN again).
    pub(crate) fn go_online(&mut self) {
        let address = std::env::var("H2_LIVE").unwrap_or_else(|_| {
            live_server(&self.menu.profile.server, || listens(HERE, HERE_WAIT))
        });
        let fakes = std::env::var("H2_LIVE_FAKE_PLAYERS")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        let profile = client::Profile {
            gamertag: self.menu.profile.name.clone(),
            look: self.menu.profile.look,
            maps: map_hashes(&self.maps),
            guests: (self.seats.len() - 1) as u8,
        };
        let now = self.online.now();
        self.online.connect(&address, profile, fakes, now);
        if let Some(why) = &self.online.failed {
            println!("live: {}", why.to_lowercase());
            self.menu.notice = Some(why.clone());
        }
    }

    pub(crate) fn sign_out(&mut self) {
        self.online.sign_out();
    }

    /// Keep up with the online service, every frame (while maps load too).
    pub(crate) fn update_live(&mut self) {
        let now = self.online.now();
        for event in self.online.poll(now) {
            match event {
                // Launchers' matches only; the server never makes one for
                // the game.
                LiveEvent::Welcomed | LiveEvent::LauncherMatch(_) => {}
                LiveEvent::Refused(why) | LiveEvent::Lost(why) => {
                    println!("live: {}", why.to_lowercase());
                    // A match's links go through the service: it's over
                    // (and so is a custom game).
                    if self.online.in_match() {
                        self.back_to_party();
                    }
                    if self.online.in_custom() {
                        self.custom_over();
                    }
                    if !online_screen(self.menu.screen) {
                        continue;
                    }
                    if why == live::GAMERTAG_TAKEN {
                        self.menu.ask_gamertag(TAKEN);
                    } else {
                        self.menu.show(Screen::Live);
                        self.menu.notice = Some(why.to_uppercase());
                    }
                    self.sound.play_ui(&self.scene, Sound::Back);
                }
                LiveEvent::Invited { from, .. } => {
                    let t = &self.online.text.players;
                    let sent = words(t, "party_invite_received", "INVITED YOU INTO THEIR PARTY");
                    self.menu.notice = Some(format!("{from} {sent}"));
                    self.sound.play_ui(&self.scene, Sound::Advance);
                }
                LiveEvent::Notice(text) => self.menu.notice = Some(text.to_uppercase()),
                LiveEvent::Match(info) => self.match_ready(info),
                LiveEvent::HostMatch(id) => self.host_match(id),
                LiveEvent::Link(link) if self.online.in_match() || self.online.in_custom() => {
                    self.online.open_leg(link, now)
                }
                LiveEvent::Link(_) => {}
                LiveEvent::Go(id) => self.match_go(id),
                LiveEvent::MatchOver(over) => self.match_over(over),
                LiveEvent::CustomOpen { party, leader, map } => {
                    self.custom_open(party, leader, map)
                }
            }
        }
        let screen = self.menu.screen;
        match self.online.follow_party(now) {
            Some(Followed::Searching) if online_screen(screen) && screen != Screen::Matchmaking => {
                self.menu.show(Screen::Matchmaking);
                self.sound.play_ui(&self.scene, Sound::Advance);
            }
            Some(Followed::Stopped) if screen == Screen::Matchmaking => {
                self.menu.show(Screen::Live);
                let view = self.online.view(now);
                if let Some(failed) = view.and_then(|v| v.search_failed()) {
                    self.menu.notice = Some(failed);
                }
            }
            Some(Followed::CustomOver) if self.online.in_custom() => {
                // Over mid-game, the host left it.
                let notice = if self.mode == Mode::Playing {
                    "THE HOST LEFT. THE CUSTOM GAME IS OVER."
                } else {
                    "THE CUSTOM GAME IS OVER"
                };
                self.custom_over();
                self.menu.notice = Some(notice.into());
                self.sound.play_ui(&self.scene, Sound::Back);
            }
            _ => {}
        }
        // Relay legs whose other end came, for the match or custom game.
        for (link, conn) in self.online.linked() {
            if self.online.in_match() {
                self.match_linked(link, conn);
            } else {
                self.custom_linked(link, conn);
            }
        }
        self.update_match();
        self.update_custom();
        self.auto_search(now);
    }

    /// For testing (H2_LIVE_AUTO=search:<key>): lead the party into that
    /// playlist's searches, each time it's been back in its lobby a while,
    /// signing in again if need be. It searches even where the menus
    /// wouldn't (the party lacks a map, say), to try the service.
    fn auto_search(&mut self, now: f64) {
        let Some(key) = &self.online.auto else {
            return;
        };
        if self.online.failed.is_some() && now - self.online.dialed >= AUTO_RETRY {
            println!("live: signing in again");
            self.menu.show(Screen::Live);
            self.go_online();
            return;
        }
        let here = self.menu.screen == Screen::Live && self.loading.is_none();
        let view = self.online.view(now).filter(|_| here);
        let lobby = |v: &OnlineView| v.party().is_some_and(|p| p.activity == Activity::Lobby);
        let ready =
            view.filter(|v| v.leads() && v.search.is_none() && v.game.is_none() && lobby(v));
        let playlist = ready.and_then(|v| {
            let p = v.playlists().iter().find(|p| p.key == *key)?;
            Some((p.id, v.missing_text(p)))
        });
        let Some((id, missing)) = playlist else {
            self.online.idle = now;
            return;
        };
        if now - self.online.idle >= AUTO_WAIT {
            self.online.idle = now;
            self.menu.show(Screen::Matchmaking);
            if let Some(missing) = missing {
                println!("live: searching anyway: {}", missing.to_lowercase());
            }
            self.ask_live(Action::Search(Some(id)));
        }
    }

    /// Carry out a menu action for the online service.
    pub(crate) fn ask_live(&mut self, action: Action) {
        let now = self.online.now();
        self.online.ask(&action, now);
    }
}

/// An online service run inside the game, with made-up players signed in
/// to it (H2_LIVE=mem), for trying the online screens without a server.
struct TestService {
    server: Server,
    /// Its data folder, removed with it.
    dir: PathBuf,
    fakes: Vec<LiveClient>,
    /// Their accounts.
    accounts: Vec<u64>,
    /// Which of them have done their part (see `poll`).
    done: Vec<bool>,
}

/// Test services started in this process, to tell their folders apart.
static TEST_SERVICES: AtomicUsize = AtomicUsize::new(0);

/// Whether the process `pid` still runs. Only Linux can say (elsewhere it
/// might, so its folder is left alone).
fn running(pid: &str) -> bool {
    !cfg!(target_os = "linux") || Path::new("/proc").join(pid).exists()
}

impl TestService {
    fn start(fakes: usize, now: f64) -> Result<TestService, String> {
        // Each in a folder of its own, named for the process; those left
        // by games that didn't stop cleanly go.
        let temp = std::env::temp_dir();
        for entry in std::fs::read_dir(&temp).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let pid = name
                .strip_prefix("h2live-mem-")
                .and_then(|n| n.split('-').next());
            if pid.is_some_and(|pid| !running(pid)) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
        let n = TEST_SERVICES.fetch_add(1, Ordering::Relaxed);
        let dir = temp.join(format!("h2live-mem-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let server = Server::open(&dir, Some("an in-game test service"))?;
        let mut service = TestService {
            server,
            dir,
            fakes: Vec::new(),
            accounts: Vec::new(),
            done: Vec::new(),
        };
        for k in 0..fakes.min(MAX_FAKES) {
            let key = client::identity(&service.dir.join(format!("fake-{k}.key")))
                .map_err(|e| e.to_string())?;
            let profile = client::Profile {
                gamertag: bot_name(k).into(),
                look: bot_look(k),
                maps: Vec::new(),
                // One plays with a splitscreen guest.
                guests: (k == 5) as u8,
            };
            // Each from an address of its own (sign-ins are limited by
            // address).
            let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, k as u8 + 1));
            let conn = service.connect(Route::Live, ip, now);
            let card = service.dir.join(format!("fake-{k}-card.txt"));
            let account = h2live::store::account_id(&key.verifying_key().to_bytes());
            let fake = LiveClient::new(conn, key, &profile, &card, now);
            service.fakes.push(fake);
            service.accounts.push(account);
            service.done.push(false);
        }
        Ok(service)
    }

    /// A new link to the service from `ip` (to sign in, or a relay leg):
    /// the PC's end.
    fn connect(&mut self, route: Route, ip: IpAddr, now: f64) -> Connection {
        let (server_end, pc_end) = Connection::pair();
        self.server.accept(server_end, route, ip, now);
        pc_end
    }

    /// Run the service and its made-up players a step. They take every
    /// invite, and once signed in each does its part: the second joins the
    /// first's party; the third makes its party invite only and invites
    /// the fourth; the fifth invites us; and the rest join the fifth's
    /// party.
    fn poll(&mut self, now: f64) {
        self.server.poll(now);
        for k in 0..self.fakes.len() {
            for event in self.fakes[k].poll(now) {
                if let LiveEvent::Invited { party, .. } = event {
                    self.fakes[k].send(ToServer::Accept(party));
                }
            }
            if self.done[k] || !self.fakes[k].signed_in() {
                continue;
            }
            let party_of = |j: usize| -> Option<u64> {
                let fake: &LiveClient = self.fakes.get(j)?;
                Some(fake.view.party.as_ref()?.id)
            };
            let mut asks = Vec::new();
            match k {
                1 => asks.extend(party_of(0).map(ToServer::JoinParty)),
                2 => {
                    let fourth = self.fakes.get(3);
                    if fourth.is_none_or(LiveClient::signed_in) {
                        asks.push(ToServer::Privacy(Privacy::InviteOnly));
                        asks.extend(self.accounts.get(3).map(|&a| ToServer::Invite(a)));
                    }
                }
                4 => {
                    let online = &self.fakes[k].view.online;
                    let us = online.iter().find(|p| !self.accounts.contains(&p.account));
                    asks.extend(us.map(|p| ToServer::Invite(p.account)));
                }
                k if k > 4 => asks.extend(party_of(4).map(ToServer::JoinParty)),
                _ => self.done[k] = true,
            }
            if !asks.is_empty() {
                self.done[k] = true;
            }
            for ask in asks {
                self.fakes[k].send(ask);
            }
        }
    }
}

impl Drop for TestService {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halo_2s_numbers_are_filled_in() {
        assert_eq!(fill("Start in %02d:%02d", &[0, 20]), "Start in 00:20");
        assert_eq!(fill("%d more needed", &[3]), "3 more needed");
        assert_eq!(fill("%2d%% complete, %s", &[7]), " 7% complete, %s");
        assert_eq!(fill("trailing %", &[]), "trailing %");
        assert_eq!(clock_text(65.9), "1:05");
    }

    #[test]
    fn the_server_is_this_pcs_or_else_the_one_chosen() {
        let here = format!("ws://{HERE}");
        let address = |chosen: &str| live_server(chosen, || false);
        assert_eq!(
            address("https://h2live.example.com/"),
            "wss://h2live.example.com"
        );
        assert_eq!(
            address(" HTTP://203.0.113.5:47050"),
            "ws://203.0.113.5:47050"
        );
        assert_eq!(address("ws://203.0.113.5:47050"), "ws://203.0.113.5:47050");
        assert_eq!(live_server("wss://h2live.example.com", || true), here);
        // Something listening on this PC is found, and nothing isn't.
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let at = listener.local_addr().unwrap();
        assert!(listens(at, HERE_WAIT));
        drop(listener);
        assert!(!listens(at, HERE_WAIT));
    }

    /// A folder of a test's own, removed afterwards.
    struct TempDir(PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn profile(gamertag: &str) -> client::Profile {
        client::Profile {
            gamertag: gamertag.into(),
            ..Default::default()
        }
    }

    /// Poll until `done` says so, a twentieth of a second at a time.
    fn until(online: &mut Online, now: &mut f64, done: impl Fn(&OnlineView) -> bool) {
        for _ in 0..400 {
            *now += 0.05;
            online.poll(*now);
            if online.view(*now).is_some_and(|v| done(&v)) {
                return;
            }
        }
        panic!("never happened");
    }

    #[test]
    fn the_in_game_service_has_players_in_parties() {
        let home = TempDir(std::env::temp_dir().join(format!("h2-online-{}", std::process::id())));
        let mut online = Online::new(LiveText::default(), Some(home.0.join("identity.key")));
        // Until asked, we're not online at all.
        assert!(online.view(0.0).is_none());
        let mut now = 0.0;
        online.connect("mem", profile("JOHN"), 6, now);
        let view = online.view(now).unwrap();
        assert!(view.live.is_none() && view.failed.is_none(), "connecting");
        // Signed in, alone in a party of our own; the fifth asks us in.
        until(&mut online, &mut now, |v| {
            v.invite().is_some() && v.others().len() == 6
        });
        assert!(home.0.join("identity.key").exists());
        let v = online.view(now).unwrap();
        assert_eq!(v.live.unwrap().welcome.as_ref().unwrap().gamertag, "JOHN");
        assert!(v.leads() && !v.partied());
        let (party, from) = v.invite().unwrap().clone();
        assert_eq!(from, bot_name(4));
        let others = v.others();
        let p = |n: usize| *others.iter().find(|p| p.gamertag == bot_name(n)).unwrap();
        // The first two together in an open party, the next two in one
        // that's invite only.
        assert_eq!(p(0).party, p(1).party);
        assert!(p(0).open && p(0).size == 2);
        assert_eq!(p(2).party, p(3).party);
        assert!(!p(2).open);
        assert_eq!(v.doing(p(2)), "IN A PARTY OF 2");
        assert_eq!(v.doing(p(0)), "IN A PARTY, 14 OPENINGS");
        let first = p(0).account;
        // Join the fifth's: it leads, and the sixth brings a guest.
        online.ask(&Action::Accept(party), now);
        until(&mut online, &mut now, |v| v.party_size() == 4);
        let v = online.view(now).unwrap();
        assert!(v.partied() && !v.leads());
        assert_eq!(v.leader(), Some(bot_name(4)));
        let leader = v.others().into_iter().find(|p| p.gamertag == bot_name(4));
        assert_eq!(v.doing(leader.unwrap()), "YOUR PARTY LEADER");
        // Our invite is taken: the first leaves its party for ours.
        online.ask(&Action::Invite(first), now);
        until(&mut online, &mut now, |v| v.party_size() == 5);
        let v = online.view(now).unwrap();
        assert_eq!(v.search_status().0, "SEARCHING FOR A GAME...");
        // Signing out leaves.
        online.sign_out();
        assert!(online.view(now).is_none());
    }

    /// A service on a port of its own, as the h2live program runs one:
    /// WebSockets that come to it are the server's, by their path.
    struct NetService {
        server: Server,
        asked: Receiver<Result<h2net::Request, String>>,
        port: u16,
        clock: Instant,
    }

    impl NetService {
        fn start(dir: &Path) -> NetService {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let (tx, asked) = mpsc::channel();
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let _ = tx.send(h2net::accept(stream, &["/live", "/link"]));
                }
            });
            NetService {
                server: Server::open(dir, Some("a test service")).unwrap(),
                asked,
                port,
                clock: Instant::now(),
            }
        }

        /// Seconds since it started.
        fn now(&self) -> f64 {
            self.clock.elapsed().as_secs_f64()
        }

        /// Take what came and run the server a step, after a moment.
        fn poll(&mut self) -> f64 {
            std::thread::sleep(Duration::from_millis(5));
            let now = self.now();
            while let Ok(Ok(h2net::Request::WebSocket(conn, path, _))) = self.asked.try_recv() {
                let route = if path == "/link" {
                    Route::Link
                } else {
                    Route::Live
                };
                self.server
                    .accept(conn, route, IpAddr::V4(Ipv4Addr::LOCALHOST), now);
            }
            self.server.poll(now);
            now
        }
    }

    #[test]
    fn services_on_the_network_are_dialed_in_the_background() {
        let home = TempDir(std::env::temp_dir().join(format!("h2-dial-{}", std::process::id())));
        let mut service = NetService::start(&home.0.join("service"));
        let mut online = Online::new(LiveText::default(), Some(home.0.join("identity.key")));
        let address = format!("ws://127.0.0.1:{}/live/", service.port);
        online.connect(&address, profile("JOHN"), 0, 0.0);
        let view = online.view(0.0).unwrap();
        assert!(view.live.is_none() && view.failed.is_none() && !view.waking);
        assert!(online.view(WAKING + 1.0).unwrap().waking, "slow to answer");
        let signed_in = (0..1000).any(|_| {
            let now = service.poll();
            online.poll(now);
            online.view(now).is_some_and(|v| v.party().is_some())
        });
        assert!(signed_in);
        // Nothing there: why comes back, to the notice line.
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("ws://127.0.0.1:{}", closed.local_addr().unwrap().port());
        drop(closed);
        online.connect(&address, profile("JOHN"), 0, 0.0);
        let failed = (0..1000).find_map(|_| {
            std::thread::sleep(Duration::from_millis(5));
            online.poll(service.now());
            online.failed.clone()
        });
        assert!(failed.unwrap().starts_with("COULDN'T CONNECT"));
        // Addresses that aren't the service's say so at once.
        online.connect("h2live.example.com", profile("JOHN"), 0, 0.0);
        let why = online.view(0.0).unwrap().failed.unwrap();
        assert_eq!(why, "NO ONLINE SERVICE AT H2LIVE.EXAMPLE.COM");
    }

    #[test]
    fn a_matchs_pcs_are_linked_through_the_service() {
        let home = TempDir(std::env::temp_dir().join(format!("h2-legs-{}", std::process::id())));
        let mut service = NetService::start(&home.0.join("service"));
        let address = format!("ws://127.0.0.1:{}", service.port);
        // Two PCs with the same Lockout, each alone in its party.
        let mut pcs: Vec<Online> = ["ALPHA", "BRAVO"]
            .iter()
            .map(|name| {
                let key = home.0.join(format!("{name}.key"));
                let mut online = Online::new(LiveText::default(), Some(key));
                let me = client::Profile {
                    maps: vec![("lockout".into(), 99)],
                    ..profile(name)
                };
                online.connect(&address, me, 0, 0.0);
                online
            })
            .collect();
        let mut linked = [Vec::new(), Vec::new()];
        let mut hosting = None;
        for _ in 0..2000 {
            let now = service.poll();
            for (k, online) in pcs.iter_mut().enumerate() {
                for event in online.poll(now) {
                    match event {
                        LiveEvent::HostMatch(id) => {
                            online.send(ToServer::Hosting(id));
                            hosting = Some(k);
                        }
                        LiveEvent::Link(link) => online.open_leg(link, now),
                        _ => {}
                    }
                }
                // Both search Head to Head.
                let lobby = online.view(now).and_then(|v| Some(v.party()?.activity));
                if lobby == Some(Activity::Lobby) && online.search.is_none() {
                    online.ask(&Action::Search(Some(1)), now);
                }
                linked[k].extend(online.linked());
            }
            // The host's two legs, and the other's one.
            if linked[0].len() + linked[1].len() == 3 {
                break;
            }
        }
        // The host has a leg to the PC joining it, and a fan-out leg; the
        // other a leg to the host. What the host sends either way, the
        // other gets.
        let host = hosting.expect("a host");
        let mut legs = std::mem::take(&mut linked[host]);
        legs.sort_by_key(|l| l.0.end == live::End::Fanout);
        let (mut fanout, mut ours) = (legs.pop().unwrap(), legs.pop().unwrap());
        let mut theirs = linked[1 - host].remove(0);
        let ends = [ours.0.end, fanout.0.end, theirs.0.end];
        assert_eq!(
            ends,
            [live::End::Host, live::End::Fanout, live::End::Joiner]
        );
        assert_eq!(theirs.0.map, "lockout");
        let mut hear = || {
            (0..1000).find_map(|_| {
                service.poll();
                theirs.1.receive().unwrap().pop()
            })
        };
        ours.1.send(9, b"hello");
        ours.1.flush().unwrap();
        assert_eq!(hear(), Some((9, b"hello".to_vec())));
        let everyone = ToServer::Fanout {
            to: Some(vec![ours.0.peer]),
            kind: 10,
            body: b"hello all".to_vec(),
        };
        everyone.send(&mut fanout.1);
        fanout.1.flush().unwrap();
        assert_eq!(hear(), Some((10, b"hello all".to_vec())));
    }

    #[test]
    fn relay_legs_go_to_the_services_link_path() {
        let url = |address| service_url(address, "link");
        assert_eq!(url("ws://127.0.0.1:47050"), "ws://127.0.0.1:47050/link");
        assert_eq!(url("wss://h2.example.com/"), "wss://h2.example.com/link");
        assert_eq!(
            url("wss://h2.example.com/live"),
            "wss://h2.example.com/link"
        );
        assert_eq!(service_url("ws://h:1/link/", "live"), "ws://h:1/live");
    }

    #[test]
    fn the_party_follows_its_searches() {
        let home = TempDir(std::env::temp_dir().join(format!("h2-follow-{}", std::process::id())));
        let mut online = Online::new(LiveText::default(), Some(home.0.join("identity.key")));
        let mut now = 0.0;
        online.connect("mem", profile("JOHN"), 0, now);
        until(&mut online, &mut now, |v| v.party().is_some());
        // Ours shows at once, but with no maps the service won't start it,
        // and says why.
        online.ask(&Action::Search(Some(2)), now);
        assert_eq!(online.view(now).unwrap().search.map(|s| s.0), Some(2));
        assert_eq!(online.follow_party(now), None, "until the service answers");
        let mut notices = Vec::new();
        let stopped = (0..100).find_map(|_| {
            now += 0.05;
            for e in online.poll(now) {
                if let LiveEvent::Notice(text) = e {
                    notices.push(text);
                }
            }
            online.follow_party(now)
        });
        assert_eq!(stopped, Some(Followed::Stopped));
        assert_eq!(
            notices,
            ["YOUR PARTY HAS NO MAP FROM THIS PLAYLIST IN COMMON"]
        );
        assert!(online.view(now).unwrap().search.is_none());
        // The leader's searches, as the party sees them: started, then
        // stopped, or on to a match.
        let set = |online: &mut Online, activity| {
            if let Link::Live(client) = &mut online.link {
                client.view.party.as_mut().unwrap().activity = activity;
            }
        };
        set(&mut online, Activity::Searching);
        assert_eq!(online.follow_party(now), Some(Followed::Searching));
        assert!(online.view(now).unwrap().search.is_some());
        assert_eq!(online.follow_party(now), None, "still searching");
        set(&mut online, Activity::Playing);
        assert_eq!(online.follow_party(now), None, "a match was found");
        assert!(online.view(now).unwrap().search.is_none());
        set(&mut online, Activity::Lobby);
        assert_eq!(online.follow_party(now), None, "back from the match");
        set(&mut online, Activity::Searching);
        online.follow_party(now);
        set(&mut online, Activity::Lobby);
        assert_eq!(
            online.follow_party(now),
            Some(Followed::Stopped),
            "cancelled"
        );
        // One the service never answers is given up on.
        online.ask(&Action::Search(None), now);
        assert_eq!(online.follow_party(now + 1.0), None);
        let given_up = online.follow_party(now + ASK_WAIT);
        assert_eq!(given_up, Some(Followed::Stopped));
        // The leader's custom games, open and then over.
        set(&mut online, Activity::Custom);
        assert_eq!(online.follow_party(now), None);
        assert_eq!(online.follow_party(now), None, "still open");
        set(&mut online, Activity::Lobby);
        assert_eq!(online.follow_party(now), Some(Followed::CustomOver));
    }

    #[test]
    fn cards_and_recent_players_are_kept_beside_their_keys() {
        let card = |key: &str| kept_with(Path::new(key), "live-card.txt", "card");
        assert_eq!(card("/h/identity.key"), Path::new("/h/live-card.txt"));
        assert_eq!(card("/h/alpha.key"), Path::new("/h/alpha-card.txt"));
        assert_eq!(card("/h/bravo"), Path::new("/h/bravo-card.txt"));
        let recent = |key: &str| kept_with(Path::new(key), "recent-players.txt", "recent");
        assert_eq!(
            recent("/h/identity.key"),
            Path::new("/h/recent-players.txt")
        );
        assert_eq!(recent("/h/alpha.key"), Path::new("/h/alpha-recent.txt"));
    }
}
