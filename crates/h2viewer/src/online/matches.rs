//! Matches the online service makes, played as Halo 2 played them: every PC
//! in one loads its map behind the pregame lobby while the service asks one
//! of them to host. The host opens a lobby the others join through relay
//! legs, and starts the game when the service says (or once they're all in
//! its lobby, if that's later). When the game ends each PC says how it saw
//! it end, and a while later everyone goes back to their party, to hear
//! what it did to their levels.

use super::online_screen;
use crate::lan::Net;
use crate::menu::{self, Screen, SeatInfo, Sound};
use crate::options::{presets, GameOptions};
use crate::scene;
use crate::{App, Mode, Then};
use h2net::live::{LinkInfo, MatchInfo, MatchOver, MatchPlayer, ToServer};
use h2net::{Client, Connection, Host, Lobby, LobbyPlayer, Verified};
use h2sim::bot::bot_name;
use h2sim::game::{guest_name, GameType, Look, TEAMS};
use h2sim::Game;

/// Seconds from a match's carnage report coming up to everyone going back
/// to their party.
const RETURN_WAIT: f64 = 15.0;
/// Longest the host waits, once the service says to start, for PCs still
/// coming into its lobby.
const HELLO_WAIT: f64 = 5.0;
/// Seconds everyone sees who they play with, in the pregame lobby, at
/// least.
const PREGAME: f64 = 8.0;

/// A match we're in, from MATCH until we're back in the party lobby.
pub(super) struct Matched {
    pub(super) info: MatchInfo,
    /// When MATCH came: the pregame countdown starts then.
    since: f64,
    /// We host it (HOST_MATCH).
    hosting: bool,
    /// When the service said to start (GO).
    go: Option<f64>,
    /// Hosting: PCs given to the host through relay legs.
    added: usize,
    /// Its game started here.
    started: bool,
    /// Hosting: the accounts of PCs that left the game.
    left: Vec<u64>,
    /// When its carnage report came up here (and we said how it ended).
    ended: Option<f64>,
    /// What the service made of it (MATCH_OVER).
    over: Option<MatchOver>,
}

impl Matched {
    fn new(info: MatchInfo, now: f64) -> Matched {
        Matched {
            info,
            since: now,
            hosting: false,
            go: None,
            added: 0,
            started: false,
            left: Vec::new(),
            ended: None,
            over: None,
        }
    }

    /// When its game starts: once the service says (by the end of the
    /// match's countdown at the latest), after the pregame.
    fn starts(&self) -> f64 {
        match self.go {
            Some(go) => go.max(self.since + PREGAME),
            None => self.since + f64::from(self.info.countdown),
        }
    }

    /// Seconds left to the pregame countdown, until it runs out.
    pub(super) fn countdown(&self, now: f64) -> Option<f64> {
        let left = self.starts() - now;
        (left > 0.0 && !self.started).then_some(left)
    }

    /// Once its carnage report is up: seconds until we're back in the party
    /// lobby.
    pub(super) fn returning(&self, now: f64) -> Option<f64> {
        self.ended.map(|t| (RETURN_WAIT - (now - t)).max(0.0))
    }
}

/// Everyone a match has: each PC's person and then their splitscreen
/// guests, with that PC's place in the match, and their names and looks.
fn roster(info: &MatchInfo) -> impl Iterator<Item = (&MatchPlayer, String, Look)> {
    info.players.iter().flat_map(|p| {
        (0..=usize::from(p.guests)).map(move |k| (p, guest_name(&p.gamertag, k), p.look.guest(k)))
    })
}

/// A match's game options: its variant's, with its time limit.
pub fn match_options(info: &MatchInfo) -> GameOptions {
    let variant = presets()
        .into_iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(&info.preset));
    GameOptions {
        time_limit: u32::from(info.time_limit),
        ..variant.map_or_else(GameOptions::default, |v| v.1)
    }
}

/// A match's lobby, as its host shows it to the PCs joining (and the
/// pregame screen shows it to everyone).
pub fn lobby_of(info: &MatchInfo) -> Lobby {
    let game_type = menu::GAME_TYPES.iter().find(|t| t.0 == info.game_type);
    let score = match info.score {
        0 => "NO LIMIT".into(),
        n => crate::local::score_text(n as i32, info.game_type.timed()),
    };
    Lobby {
        map: info.map.clone(),
        game_type: game_type.map_or("", |t| t.1).into(),
        score,
        options: menu::options_label(&match_options(info)),
        teams: info.game_type.teams(),
        players: roster(info)
            .map(|(p, name, look)| LobbyPlayer {
                name,
                look,
                team: p.team,
                remote: p.account != info.host,
            })
            .collect(),
        bots: info.bots,
    }
}

/// A match's bots, as its host seats them: each on the team with the
/// fewest players so far (everyone's guests and the bots before it
/// counted), and each by a bot name no one in the match goes by, so every
/// PC tells people from bots by name. Each is its team and the number of
/// its name (see `bot_name`).
fn bots_of(info: &MatchInfo) -> Vec<(u8, usize)> {
    let mut sizes = [0; TEAMS as usize];
    let mut taken = Vec::new();
    for (p, name, _) in roster(info) {
        if let Some(n) = sizes.get_mut(usize::from(p.team)) {
            *n += 1;
        }
        taken.push(name);
    }
    let free = |k: &usize| !taken.iter().any(|n| n == bot_name(*k));
    let mut names = (0..scene::MAX_BODIES).filter(free).chain(0..);
    (0..info.bots)
        .map(|_| {
            let team = (0..TEAMS)
                .min_by_key(|&t| sizes[usize::from(t)])
                .unwrap_or(0);
            sizes[usize::from(team)] += 1;
            (team, names.next().unwrap_or(0))
        })
        .collect()
}

/// The Spartan each PC's person played in `game`, by their account:
/// known by their gamertag (which no bot has; see `bots_of`).
fn players_of(info: &MatchInfo, game: &Game) -> Vec<(u64, usize)> {
    info.players
        .iter()
        .filter_map(|p| {
            let i = (0..game.players.len()).find(|&i| game.name(i) == p.gamertag)?;
            Some((p.account, i))
        })
        .collect()
}

/// What a match did to our level in `playlist` (by name), or why it
/// didn't count.
fn over_notice(over: &MatchOver, playlist: &str) -> Option<String> {
    if !over.reason.is_empty() {
        return Some(over.reason.clone());
    }
    let &(_, old, new) = over.levels.first()?;
    Some(match new.cmp(&old) {
        std::cmp::Ordering::Greater => format!("LEVEL UP! YOU'RE LEVEL {new} IN {playlist}"),
        std::cmp::Ordering::Less => format!("LEVEL DOWN. YOU'RE LEVEL {new} IN {playlist}"),
        std::cmp::Ordering::Equal => format!("STILL LEVEL {new} IN {playlist}"),
    })
}

impl App {
    /// A match is ready (MATCH): load its map behind the pregame lobby.
    /// It comes again, for the match we're in, when another PC is asked to
    /// host it.
    pub(super) fn match_ready(&mut self, info: MatchInfo) {
        let now = self.online.now();
        if let Some(m) = self
            .online
            .matched
            .as_mut()
            .filter(|m| m.info.id == info.id)
        {
            m.info = info;
            return;
        }
        println!("live: a match on {}", info.map);
        // Whatever was going on here stops for it.
        self.net = Net::Offline;
        if self.mode == Mode::Playing {
            self.back_to_lobby();
        }
        let map = info.map.clone();
        self.online.matched = Some(Matched::new(info, now));
        self.menu.show(Screen::Pregame);
        self.menu.notice = None;
        self.sound.play_ui(&self.scene, Sound::Advance);
        if let Some(k) = self
            .maps
            .iter()
            .position(|m| m.name.eq_ignore_ascii_case(&map))
        {
            self.menu.settings.map = k;
        }
        if map.eq_ignore_ascii_case(&self.map_name) {
            return;
        }
        match self.map_named(&map) {
            Some(path) => self.begin_load(path, Then::Match),
            None => {
                self.quit_match(false);
                self.menu.notice = Some(format!("YOU DON'T HAVE {}", menu::map_title(&map)));
            }
        }
    }

    /// Host the match (HOST_MATCH): PCs joining wait in its lobby until
    /// its game starts.
    pub(super) fn host_match(&mut self, id: u64) {
        let Some(m) = self.online.matched.as_mut().filter(|m| m.info.id == id) else {
            return;
        };
        m.hosting = true;
        let mut host = Host::online(&m.info.map);
        host.set_lobby(lobby_of(&m.info));
        self.net = Net::Hosting(host);
        self.online.send(ToServer::Hosting(id));
        println!("live: hosting the match");
    }

    /// The service said to start the match's game (GO).
    pub(super) fn match_go(&mut self, id: u64) {
        let now = self.online.now();
        if let Some(m) = self.online.matched.as_mut().filter(|m| m.info.id == id) {
            m.go = Some(now);
        }
    }

    /// A relay leg's other end came: as the host, a PC joining; otherwise,
    /// the host, whose lobby we wait in until its game starts.
    pub(super) fn match_linked(&mut self, link: LinkInfo, conn: Connection) {
        let gamertag = self.online.me().map(|(_, g)| g.to_string());
        let Some(m) = self
            .online
            .matched
            .as_mut()
            .filter(|m| m.info.id == link.id)
        else {
            return;
        };
        println!("live: linked to {}", link.gamertag);
        if !link.joiner {
            if let Net::Hosting(host) = &mut self.net {
                let verified = Verified {
                    account: link.peer,
                    gamertag: link.gamertag,
                    level: link.level,
                    team: link.team,
                };
                host.add_connection(conn, verified);
                m.added += 1;
            }
            return;
        }
        let Some(gamertag) = gamertag else {
            return;
        };
        // The host knows our team (and gamertag) from the service.
        let teams = self.wanted_teams();
        let me = (gamertag.as_str(), self.menu.profile.look);
        let client = Client::over(conn, &self.game, &link.map, &teams, me);
        self.net = Net::Joined {
            client,
            computer: link.gamertag,
            seated: false,
            waiting_pads: Vec::new(),
            lobby: None,
            teams_sent: teams,
        };
    }

    /// Hosting: start the match's game, everyone here on the team the
    /// service gave us and known by our gamertag there.
    fn start_match(&mut self) {
        let Some(m) = &self.online.matched else {
            return;
        };
        let info = m.info.clone();
        let account = self.online.me().map_or(0, |(a, _)| a);
        let mine = info.players.iter().find(|p| p.account == account);
        for seat in &mut self.seats {
            seat.team = mine.map_or(0, |p| p.team);
        }
        self.seat_players(&bots_of(&info), &match_options(&info));
        if let Some(p) = mine {
            for (k, l) in self.locals.iter().enumerate() {
                self.game.set_name(l.player, &guest_name(&p.gamertag, k));
            }
        }
        if let Net::Hosting(host) = &mut self.net {
            host.start(&self.map_name);
        }
        self.lan_wait = 0.0;
        if let Some(m) = &mut self.online.matched {
            m.started = true;
        }
        println!("live: the match's game started");
    }

    /// Keep the match going, every frame: start the game when it's time
    /// (hosting), say how it ended, and go back to the party a while after.
    pub(super) fn update_match(&mut self) {
        let now = self.online.now();
        let Some(m) = &self.online.matched else {
            return;
        };
        let all_in = match &self.net {
            Net::Hosting(host) => host.joined() >= m.added,
            _ => false,
        };
        let loaded = self.loading.is_none() && self.map_name.eq_ignore_ascii_case(&m.info.map);
        let start = m.hosting
            && !m.started
            && loaded
            && m.go.is_some_and(|go| all_in || now - go >= HELLO_WAIT)
            && now >= m.starts();
        if start {
            self.start_match();
        }
        let seated = matches!(self.net, Net::Joined { seated: true, .. });
        let over = self.mode == Mode::Playing
            && self.game.over()
            && self.game_over.is_some_and(|t| t >= crate::GAME_OVER_DELAY);
        let Some(m) = &mut self.online.matched else {
            return;
        };
        m.started |= seated;
        let mut result = None;
        if m.started && m.ended.is_none() && over {
            m.ended = Some(now);
            let players = players_of(&m.info, &self.game);
            let players = h2live::client::results(&self.game, &players, &m.left);
            result = Some(ToServer::Result {
                id: m.info.id,
                players,
            });
        }
        let back = m.ended.is_some_and(|t| now - t >= RETURN_WAIT);
        if let Some(result) = result {
            println!("live: the game is over");
            self.online.send(result);
            if let Net::Hosting(host) = &self.net {
                let rate = host.sent() as f64 * 8.0 / 1000.0 / self.game.time.max(1.0);
                println!(
                    "live: sent {} bytes of the game, {rate:.0} kbit/s",
                    host.sent()
                );
            }
        }
        if back {
            self.back_to_party();
        }
    }

    /// The service's word on a match (MATCH_OVER): back to the party at
    /// once, unless its game ended here (then a while after).
    pub(super) fn match_over(&mut self, over: MatchOver) {
        println!(
            "live: the match is over{}: {}",
            if over.counted { ", counted" } else { "" },
            over.reason.to_lowercase()
        );
        let Some(m) = self
            .online
            .matched
            .as_mut()
            .filter(|m| m.info.id == over.id)
        else {
            // We're back in the party already.
            if online_screen(self.menu.screen) {
                self.menu.notice = self.over_notice(&over);
            }
            return;
        };
        let ended = m.ended.is_some();
        m.over = Some(over);
        if !ended {
            self.back_to_party();
        }
    }

    /// What a match did to our level, or why it didn't count.
    fn over_notice(&self, over: &MatchOver) -> Option<String> {
        let now = self.online.now();
        let playlist = over.levels.first().map(|l| l.0);
        let name = playlist.and_then(|p| Some(self.online.view(now)?.playlist_name(p)));
        over_notice(over, &name.unwrap_or_default())
    }

    /// Leave the match: the service hears we quit (or lost the host),
    /// unless its game ended here already.
    pub(crate) fn quit_match(&mut self, host_lost: bool) {
        let m = self.online.matched.as_ref();
        let going = m.filter(|m| m.ended.is_none() && m.over.is_none());
        if let Some(id) = going.map(|m| m.info.id) {
            self.online.send(ToServer::LeftMatch { id, host_lost });
        }
        self.back_to_party();
    }

    /// Back to the party lobby from a match, closing its game, to hear
    /// what it did to our level.
    pub(crate) fn back_to_party(&mut self) {
        let m = self.online.matched.take();
        self.online.legs.clear();
        self.net = Net::Offline;
        if self.mode == Mode::Playing {
            self.back_to_lobby();
        }
        self.menu.show(Screen::Live);
        self.menu.notice = m.and_then(|m| m.over).and_then(|o| self.over_notice(&o));
        self.online.send(ToServer::Back);
        println!("live: back in the party");
    }

    /// Hosting a match (whose lobby stays up through the pregame).
    pub(crate) fn hosting_match(&self) -> bool {
        let hosting = self.online.matched.as_ref().is_some_and(|m| m.hosting);
        hosting && matches!(self.net, Net::Hosting(_))
    }

    /// The match's game ended here (so the host going is no surprise).
    pub(crate) fn match_game_over(&self) -> bool {
        self.online.in_match() && self.mode == Mode::Playing && self.game.over()
    }

    /// The match's lobby, while in one.
    pub(crate) fn match_lobby(&self) -> Option<Lobby> {
        Some(lobby_of(&self.online.matched.as_ref()?.info))
    }

    /// Everyone in the match, for the pregame lobby: their levels too.
    pub(crate) fn match_seats(&self) -> Option<Vec<SeatInfo>> {
        let m = self.online.matched.as_ref()?;
        let seats = roster(&m.info).map(|(p, name, look)| SeatInfo {
            name,
            look,
            how: "ONLINE",
            team: p.team,
            level: Some(p.level),
        });
        Some(seats.collect())
    }

    /// A player's level in the match, by name: ours as the match left it,
    /// once the service says.
    pub(crate) fn match_level(&self, name: &str) -> Option<u8> {
        let m = self.online.matched.as_ref()?;
        let (p, ..) = roster(&m.info).find(|(_, n, _)| n == name)?;
        let ours = self.online.me().is_some_and(|(a, _)| a == p.account);
        let after = m.over.as_ref().and_then(|o| o.levels.first());
        Some(after.filter(|_| ours).map_or(p.level, |l| l.2))
    }

    /// The match's game type and score to win, while in one.
    pub(crate) fn match_rules(&self) -> Option<(GameType, u32)> {
        let info = &self.online.matched.as_ref()?.info;
        Some((info.game_type, info.score))
    }

    /// Hosting: a PC left the match's game.
    pub(crate) fn match_left(&mut self, computer: &str) {
        let Some(m) = self.online.matched.as_mut().filter(|m| m.ended.is_none()) else {
            return;
        };
        let p = m.info.players.iter().find(|p| p.gamertag == computer);
        m.left.extend(p.map(|p| p.account));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(account: u64, gamertag: &str, team: u8, level: u8) -> MatchPlayer {
        MatchPlayer {
            account,
            gamertag: gamertag.into(),
            look: Look::default(),
            level,
            team,
            party: account,
            guests: 0,
        }
    }

    fn double_team() -> MatchInfo {
        MatchInfo {
            id: 7,
            playlist: 2,
            ranked: true,
            map: "lockout".into(),
            hash: 0,
            game_type: GameType::TeamSlayer,
            preset: "ROCKETS".into(),
            score: 25,
            time_limit: 600,
            bots: 0,
            host: 1,
            countdown: 20,
            players: vec![
                player(1, "ALPHA", 0, 3),
                player(2, "BRAVO", 0, 1),
                player(3, "CHARLIE", 1, 2),
                MatchPlayer {
                    guests: 1,
                    ..player(4, "DELTA", 1, 1)
                },
            ],
        }
    }

    #[test]
    fn a_match_is_played_as_its_variant_says() {
        let info = double_team();
        let options = match_options(&info);
        assert_eq!(options.preset(), Some(2), "rockets");
        assert_eq!(options.time_limit, 600);
        let unknown = MatchInfo {
            preset: "HARDCORE".into(),
            ..double_team()
        };
        assert_eq!(match_options(&unknown).preset(), Some(0));
        let lobby = lobby_of(&info);
        assert_eq!(lobby.game_type, "TEAM SLAYER");
        assert_eq!(lobby.score, "25");
        assert_eq!(lobby.options, "ROCKETS, 10 MIN");
        assert!(lobby.teams);
        // Each PC's person, then its guests; only the host's are local.
        let names: Vec<&str> = lobby.players.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "ALPHA",
                "BRAVO",
                "CHARLIE",
                "DELTA",
                &guest_name("DELTA", 1)
            ]
        );
        assert!(!lobby.players[0].remote && lobby.players[1].remote);
        assert_eq!(lobby.players[4].team, 1);
    }

    /// A match's game as its host seats it: its own person, then its
    /// bots, then each PC joining, on the team the service gave it.
    fn seated(info: &MatchInfo) -> Game {
        let mut game = h2sim::testing::game();
        game.rules.game_type = info.game_type;
        let (host, joiners): (Vec<_>, Vec<_>) =
            info.players.iter().partition(|p| p.account == info.host);
        let seat = |game: &mut Game, p: &MatchPlayer| {
            for k in 0..=usize::from(p.guests) {
                let i = game.add_player_on(p.team);
                game.set_name(i, &guest_name(&p.gamertag, k));
            }
        };
        host.into_iter().for_each(|p| seat(&mut game, p));
        for (team, k) in bots_of(info) {
            let i = game.add_player_on(team);
            game.set_name(i, bot_name(k));
        }
        joiners.into_iter().for_each(|p| seat(&mut game, p));
        game
    }

    fn team_sizes(game: &Game) -> [usize; 2] {
        let on = |t| game.players.iter().filter(|p| p.team == t).count();
        [on(0), on(1)]
    }

    #[test]
    fn bots_even_out_the_teams_the_service_made() {
        // Two against one, and a bot: on the one's team, though the host
        // is alone when it seats its bots.
        let info = MatchInfo {
            bots: 1,
            players: vec![
                player(1, "ALPHA", 1, 1),
                player(2, "KILO", 0, 1),
                player(3, "LIMA", 0, 1),
            ],
            ..double_team()
        };
        assert_eq!(bots_of(&info), [(1, 0)]);
        assert_eq!(team_sizes(&seated(&info)), [2, 2]);
        // Big Team Battle: nine people and three bots make six a side,
        // guests counted.
        let mut players: Vec<_> = (0..8)
            .map(|k| player(k + 1, "", (k % 2) as u8, 1))
            .collect();
        players[0].guests = 1;
        for (k, p) in players.iter_mut().enumerate() {
            p.gamertag = format!("PLAYER{k}");
        }
        let info = MatchInfo {
            bots: 3,
            players,
            ..double_team()
        };
        let teams: Vec<u8> = bots_of(&info).iter().map(|b| b.0).collect();
        assert_eq!(teams, [1, 0, 1]);
        assert_eq!(team_sizes(&seated(&info)), [6, 6]);
    }

    #[test]
    fn people_named_like_bots_are_told_from_them() {
        // VIPER and SARGE are people here, so no bot is.
        let info = MatchInfo {
            bots: 1,
            players: vec![
                player(1, "ALPHA", 0, 1),
                player(2, "SARGE", 0, 1),
                player(3, "VIPER", 1, 1),
            ],
            ..double_team()
        };
        let bots = bots_of(&info);
        assert_eq!(bots.len(), 1);
        assert!(!["ALPHA", "SARGE", "VIPER"].contains(&bot_name(bots[0].1)));
        let mut game = seated(&info);
        // VIPER joined last.
        let viper = game.players.len() - 1;
        assert_eq!(game.name(viper), "VIPER");
        game.players[viper].kills = 10;
        game.players[viper].score = 10;
        game.winning_team = Some(1);
        let played = players_of(&info, &game);
        assert_eq!(played.len(), 3);
        let results = h2live::client::results(&game, &played, &[]);
        let viper = results.iter().find(|r| r.account == 3).unwrap();
        assert_eq!((viper.team, viper.place, viper.kills), (1, 0, 10));
        let sarge = results.iter().find(|r| r.account == 2).unwrap();
        assert_eq!((sarge.team, sarge.place), (0, 1));
    }

    #[test]
    fn the_pregame_counts_down_to_the_start() {
        let mut m = Matched::new(double_team(), 100.0);
        // Until the service says go, to the countdown's end.
        assert_eq!(m.countdown(105.0), Some(15.0));
        assert_eq!(m.countdown(130.0), None, "waiting for maps");
        assert_eq!(m.returning(130.0), None);
        // Going at once, it still shows everyone who's playing a while.
        m.go = Some(101.0);
        assert_eq!(m.countdown(103.0), Some(5.0));
        m.go = Some(111.0);
        assert_eq!(m.countdown(110.5), Some(0.5));
        assert_eq!(m.countdown(111.0), None);
        m.ended = Some(200.0);
        assert_eq!(m.returning(205.0), Some(10.0));
        assert_eq!(m.returning(300.0), Some(0.0));
    }

    #[test]
    fn levels_won_and_lost_are_told() {
        let over = |reason: &str, levels| MatchOver {
            id: 7,
            counted: reason.is_empty(),
            reason: reason.into(),
            card: String::new(),
            levels,
        };
        let told = |o: MatchOver| over_notice(&o, "DOUBLE TEAM");
        assert_eq!(
            told(over("", vec![(2, 1, 2)])).as_deref(),
            Some("LEVEL UP! YOU'RE LEVEL 2 IN DOUBLE TEAM")
        );
        assert_eq!(
            told(over("", vec![(2, 5, 4)])).as_deref(),
            Some("LEVEL DOWN. YOU'RE LEVEL 4 IN DOUBLE TEAM")
        );
        assert_eq!(
            told(over("", vec![(2, 3, 3)])).as_deref(),
            Some("STILL LEVEL 3 IN DOUBLE TEAM")
        );
        let left = "THE HOST LEFT. THE GAME DIDN'T COUNT.";
        assert_eq!(told(over(left, Vec::new())).as_deref(), Some(left));
        assert_eq!(told(over("", Vec::new())), None, "unranked");
    }
}
