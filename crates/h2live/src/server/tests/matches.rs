//! Matchmaking and matches through the server: searching, hosting, the
//! relay, real games on the test floor played by bots, results and levels.

use super::*;
use crate::client::{results, RelayLeg};
use crate::levels::{min_xp, Placed};
use h2net::live::{LinkInfo, MatchInfo, MatchOver, Stage, QUICKMATCH};
use h2net::{Client, ClientEvent, Host, HostEvent, Lobby, Verified};
use h2sim::testing::{floor, game};
use h2sim::{Bot, Command, Game, GameType, NavGraph};

/// The playlists these tests search: some of Halo 2's, with short games on
/// the maps every test PC has.
const PLAYLISTS: &str = "\
playlist 0 ffa Rumble Pit
ranked yes
humans 3 3
party 1
variant slayer default 3 600
maps lockout midship

playlist 2 double_team Double Team
ranked yes
humans 4 4
party 2
variant team_slayer default 5 600
maps lockout midship

playlist 5 big_team Big Team Battle
humans 2 16
party 8
guests yes
bots fill 12
variant team_slayer default 25 900
maps lockout midship
";

const RUMBLE_PIT: u8 = 0;
const BIG_TEAM: u8 = 5;

const GAMERTAGS: [&str; 6] = ["ALPHA", "BRAVO", "CHARLIE", "DELTA", "ECHO", "FOXTROT"];

/// A PC playing matches as the server says: hosting them, or joining the
/// host through the relay. Bots play every Spartan in the games it hosts.
#[derive(Default)]
struct Gamer {
    /// What it does wrong: never hosts, never links, or reports results
    /// the other way round.
    wont_host: bool,
    wont_link: bool,
    lies: bool,
    /// The match it's in (MATCH).
    info: Option<MatchInfo>,
    /// The custom game it's in: its leader and map (CUSTOM_OPEN).
    custom: Option<(u64, String)>,
    legs: Vec<RelayLeg>,
    /// Why legs failed, if the server gave up on them.
    failed_legs: Vec<String>,
    hosted: Option<Hosted>,
    joined: Option<Joined>,
    /// It said how the match went, or that it left.
    reported: bool,
    over: Vec<MatchOver>,
}

/// A game this PC hosts.
struct Hosted {
    host: Host,
    game: Game,
    map: String,
    bots: Vec<Bot>,
    /// Each PC's account and Spartan, and the accounts of PCs that left.
    players: Vec<(u64, usize)>,
    left: Vec<u64>,
    started: bool,
}

/// The host's game, as this PC joined it.
struct Joined {
    client: Client,
    game: Game,
    team: u8,
    lost: bool,
}

/// The test floor's game, played as a match says.
fn game_of(game_type: GameType, score: u32, time_limit: u16) -> Game {
    let mut g = game();
    g.rules.game_type = game_type;
    g.rules.score_to_win = score;
    g.rules.time_limit = u32::from(time_limit);
    g
}

/// What custom games play.
fn custom_game() -> Game {
    game_of(GameType::Slayer, 3, 0)
}

fn lobby(map: &str) -> Lobby {
    Lobby {
        map: map.to_string(),
        ..Lobby::default()
    }
}

impl Hosted {
    /// Host `game` online, waiting in the lobby on `map`, with `me` (an
    /// account) playing as `player`.
    fn new(map: &str, game: Game, me: u64, player: usize) -> Hosted {
        let mut host = Host::online(map);
        // The tests' clock runs far faster than real time: send the game
        // every tick.
        host.set_rate(0);
        host.set_lobby(lobby(map));
        Hosted {
            host,
            game,
            map: map.to_string(),
            bots: Vec::new(),
            players: vec![(me, player)],
            left: Vec::new(),
            started: false,
        }
    }

    fn start(&mut self) {
        self.host.start(&self.map);
        self.started = true;
    }

    /// A tick of the game, with bots playing everyone.
    fn tick(&mut self, floor: &h2sim::World, nav: &NavGraph) {
        while self.bots.len() < self.game.players.len() {
            self.bots.push(Bot::new(self.bots.len() as u32 * 31 + 5));
        }
        let commands: Vec<Command> = (0..self.game.players.len())
            .map(|p| self.bots[p].think(&self.game, floor, nav, p))
            .collect();
        self.game.step(floor, &commands);
        let events = std::mem::take(&mut self.game.events);
        self.host.send(&self.game, &events, true);
    }
}

/// The server, PCs signed in to it playing matches, and the floor their
/// games are on.
struct Arena {
    w: World,
    floor: h2sim::World,
    nav: NavGraph,
    gamers: Vec<Gamer>,
    /// Each PC's events acted on so far.
    seen: Vec<usize>,
}

/// Give PCs numbered from 1 accounts at `levels` in the playlist `key`, as
/// if they had played there.
fn at_levels(w: &mut World, key: &str, levels: &[u8]) {
    let accounts: Vec<Account> = (1..)
        .zip(levels)
        .map(|(n, &level)| {
            let mut a = Account::new(key_of(n), 1_600_000_000);
            a.gamertag = GAMERTAGS[usize::from(n) - 1].into();
            a.look = profile("").look;
            a.stats.push(Stats {
                playlist: key.into(),
                rank: Rank {
                    xp: min_xp(level),
                    level,
                },
                games: 10,
                wins: 5,
            });
            a
        })
        .collect();
    store::save(&w.data.0.join("accounts.txt"), &accounts).unwrap();
}

fn key_of(n: u8) -> [u8; 32] {
    key(n).verifying_key().to_bytes()
}

impl Arena {
    /// A server with the test playlists and `pcs` PCs signed in, each in a
    /// party of its own; with levels in the playlist `key`, if given.
    fn new(name: &str, pcs: usize, levels: Option<(&str, &[u8])>) -> Arena {
        let mut w = World::new(name);
        std::fs::write(w.data.0.join("playlists.txt"), PLAYLISTS).unwrap();
        if let Some((key, levels)) = levels {
            at_levels(&mut w, key, levels);
        }
        w.restart();
        let points: Vec<glam::Vec3> = (-4..=4)
            .flat_map(|x| {
                (-4..=4).map(move |y| glam::Vec3::new(x as f32 * 3.0, y as f32 * 3.0, 0.0))
            })
            .collect();
        let floor = floor();
        let nav = NavGraph::build(&floor, &points);
        let mut arena = Arena {
            w,
            floor,
            nav,
            gamers: Vec::new(),
            seen: Vec::new(),
        };
        for n in 1..=pcs {
            arena.sign_in(n as u8);
        }
        arena
    }

    /// PC number `n` signs in, alone.
    fn sign_in(&mut self, n: u8) -> usize {
        let i = self.w.sign_in(n, GAMERTAGS[usize::from(n) - 1]);
        assert_eq!(i, self.gamers.len());
        self.gamers.push(Gamer::default());
        self.seen.push(0);
        i
    }

    fn id(&self, i: usize) -> u64 {
        self.w.id(i)
    }

    /// The PC signed in as `account`.
    fn pc(&self, account: u64) -> usize {
        (0..self.gamers.len())
            .find(|&i| self.id(i) == account)
            .unwrap()
    }

    fn send(&mut self, i: usize, message: ToServer) {
        self.w.send(i, message);
    }

    /// Everyone's PCs move on by one step: the server's, then each PC's
    /// link to it (unless frozen), then the games.
    fn step(&mut self) {
        self.w.step();
        for i in 0..self.gamers.len() {
            let events = self.w.events[i][self.seen[i]..].to_vec();
            self.seen[i] = self.w.events[i].len();
            for event in events {
                self.act(i, event);
            }
            self.play(i);
        }
    }

    /// Run until `done`, or fail after `seconds` (on the server's clock).
    fn until(&mut self, seconds: f64, done: impl Fn(&Arena) -> bool) {
        let end = self.w.now + seconds;
        while !done(self) {
            assert!(self.w.now < end, "timed out");
            self.step();
        }
    }

    fn run(&mut self, seconds: f64) {
        let end = self.w.now + seconds;
        while self.w.now < end {
            self.step();
        }
    }

    /// Do what the server said.
    fn act(&mut self, i: usize, event: LiveEvent) {
        let me = self.id(i);
        let g = &mut self.gamers[i];
        match event {
            LiveEvent::Match(info) => {
                if g.info.as_ref().map(|m| m.id) != Some(info.id) {
                    g.reported = false;
                }
                g.info = Some(info);
            }
            LiveEvent::HostMatch(id) if !g.wont_host => {
                let info = g.info.clone().unwrap();
                assert_eq!(info.id, id);
                let mine = info.players.iter().find(|p| p.account == me).unwrap();
                let mut game = game_of(info.game_type, info.score, info.time_limit);
                let player = game.add_player_on(mine.team);
                game.set_name(player, &mine.gamertag);
                for _ in 0..info.bots {
                    game.add_player();
                }
                g.hosted = Some(Hosted::new(&info.map, game, me, player));
                self.send(i, ToServer::Hosting(id));
            }
            LiveEvent::Link(link) if !g.wont_link => {
                let (server_end, pc_end) = Connection::pair();
                let w = &mut self.w;
                w.server
                    .accept(server_end, Route::Link, ip(i as u8 + 1), w.now);
                let leg = w.pcs[i].open_leg(link, pc_end);
                self.gamers[i].legs.push(leg);
            }
            LiveEvent::Go(id) => {
                assert_eq!(g.info.as_ref().map(|m| m.id), Some(id));
                if let Some(h) = &mut g.hosted {
                    h.start();
                }
            }
            LiveEvent::MatchOver(over) => {
                g.over.push(over);
                g.info = None;
                g.hosted = None;
                g.joined = None;
                g.legs.clear();
            }
            LiveEvent::CustomOpen { leader, map, .. } => {
                if leader == me {
                    match &mut g.hosted {
                        Some(h) => {
                            h.map = map.clone();
                            h.host.set_lobby(lobby(&map));
                        }
                        None => {
                            let mut game = custom_game();
                            let player = game.add_player();
                            game.set_name(player, GAMERTAGS[i]);
                            g.hosted = Some(Hosted::new(&map, game, me, player));
                        }
                    }
                }
                g.custom = Some((leader, map));
            }
            _ => {}
        }
    }

    /// Play: take relay legs that are joined, run the games, and report how
    /// they ended.
    fn play(&mut self, i: usize) {
        let Arena {
            w,
            floor,
            nav,
            gamers,
            ..
        } = self;
        let g = &mut gamers[i];
        let mut k = 0;
        while k < g.legs.len() {
            match g.legs[k].poll() {
                Ok(None) => k += 1,
                Ok(Some(conn)) => {
                    let link = g.legs.remove(k).link;
                    if let Some(h) = &mut g.hosted {
                        let verified = Verified {
                            account: link.peer,
                            gamertag: link.gamertag,
                            level: link.level,
                            team: link.team,
                        };
                        h.host.add_connection(conn, verified);
                    } else {
                        let game = match &g.info {
                            Some(m) => game_of(m.game_type, m.score, m.time_limit),
                            None => custom_game(),
                        };
                        let me = (GAMERTAGS[i], profile("").look);
                        let client = Client::over(conn, &game, &link.map, &[link.team], me);
                        g.joined = Some(Joined {
                            client,
                            game,
                            team: link.team,
                            lost: false,
                        });
                    }
                }
                Err(why) => {
                    g.legs.remove(k);
                    g.failed_legs.push(why);
                }
            }
        }
        let account = |gamertag: &str| {
            let players = g.info.as_ref().map_or(&[][..], |m| &m.players);
            players
                .iter()
                .find(|p| p.gamertag == gamertag)
                .map(|p| p.account)
        };
        let mut result = None;
        if let Some(h) = &mut g.hosted {
            for event in h.host.poll(&mut h.game, 16) {
                match event {
                    HostEvent::Joined { computer, players } => {
                        if let Some(a) = account(&computer) {
                            h.players.push((a, players[0]));
                        }
                    }
                    HostEvent::Left { computer, .. } => h.left.extend(account(&computer)),
                    _ => {}
                }
            }
            if h.started && !h.game.over() {
                h.tick(floor, nav);
            }
            if h.started && h.game.over() {
                result = Some(results(&h.game, &h.players, &h.left));
            }
        }
        if let Some(j) = &mut g.joined {
            let (events, _) = j.client.poll(&mut j.game);
            for event in events {
                match event {
                    ClientEvent::Start(map) => j.client.rejoin(&j.game, &map, &[j.team]),
                    ClientEvent::Lost(_) | ClientEvent::Refused(_) => j.lost = true,
                    _ => {}
                }
            }
            if j.client.in_game && j.game.over() {
                let players = g.info.as_ref().map_or(&[][..], |m| &m.players);
                let players: Vec<(u64, usize)> = players
                    .iter()
                    .filter_map(|p| {
                        let spartan = j.game.players.iter().position(|s| s.name == p.gamertag);
                        Some((p.account, spartan?))
                    })
                    .collect();
                result = Some(results(&j.game, &players, &[]));
            }
        }
        let Some(id) = g.info.as_ref().map(|m| m.id) else {
            return;
        };
        if g.reported {
            return;
        }
        if g.joined.as_ref().is_some_and(|j| j.lost) {
            g.reported = true;
            w.send(
                i,
                ToServer::LeftMatch {
                    id,
                    host_lost: true,
                },
            );
        } else if let Some(mut players) = result {
            if g.lies {
                for p in &mut players {
                    p.place = u8::from(p.place == 0);
                }
            }
            g.reported = true;
            w.send(i, ToServer::Result { id, players });
        }
    }

    /// Every PC searches `playlist` alone.
    fn search(&mut self, playlist: u8) {
        for i in 0..self.gamers.len() {
            self.send(i, ToServer::Search(playlist));
        }
    }

    /// Run until a match forms, and return it as everyone sees it.
    fn formed(&mut self) -> MatchInfo {
        self.until(120.0, |a| a.gamers.iter().any(|g| g.info.is_some()));
        // Everyone in it hears in the same step.
        let info = self.gamers.iter().find_map(|g| g.info.clone()).unwrap();
        for p in &info.players {
            assert_eq!(self.gamers[self.pc(p.account)].info, Some(info.clone()));
        }
        info
    }

    /// The game PC `i` hosts.
    fn hosted(&self, i: usize) -> Option<&Hosted> {
        self.gamers[i].hosted.as_ref()
    }

    /// Run until match `m`'s game has gone on for a few seconds with
    /// `players` Spartans in it.
    fn playing(&mut self, m: &MatchInfo, players: usize) {
        let host = self.pc(m.host);
        self.until(60.0, |a| {
            a.hosted(host)
                .is_some_and(|h| h.started && h.game.players.len() == players && h.game.time > 3.0)
        });
    }

    /// PC `i` quits its match through the menu.
    fn quit(&mut self, i: usize) {
        let id = self.gamers[i].info.as_ref().unwrap().id;
        self.send(
            i,
            ToServer::LeftMatch {
                id,
                host_lost: false,
            },
        );
        let g = &mut self.gamers[i];
        g.reported = true;
        g.hosted = None;
        g.joined = None;
        g.legs.clear();
    }

    /// Run until everyone in match `info` has heard it's over.
    fn finish(&mut self, info: &MatchInfo) {
        let id = info.id;
        let pcs: Vec<usize> = info.players.iter().map(|p| self.pc(p.account)).collect();
        self.until(300.0, |a| {
            pcs.iter()
                .all(|&i| a.gamers[i].over.iter().any(|o| o.id == id))
        });
    }

    /// How match `id` ended for PC `i`.
    fn over(&self, i: usize, id: u64) -> &MatchOver {
        self.gamers[i].over.iter().find(|o| o.id == id).unwrap()
    }

    /// PC `i`'s account as the server keeps it.
    fn account(&self, i: usize) -> &Account {
        self.w.server.account(self.id(i)).unwrap()
    }

    /// PC `i`'s XP and level in the playlist `key`.
    fn rank(&self, i: usize, key: &str) -> Rank {
        self.account(i)
            .stats(key)
            .map_or(Rank::default(), |s| s.rank)
    }

    fn games_log(&self) -> String {
        std::fs::read_to_string(self.w.data.0.join("games.log")).unwrap_or_default()
    }

    /// The latest link PC `i` was given to `peer` (an account).
    fn link(&self, i: usize, peer: u64) -> Option<LinkInfo> {
        self.w.events[i].iter().rev().find_map(|e| match e {
            LiveEvent::Link(link) if link.peer == peer => Some(link.clone()),
            _ => None,
        })
    }

    /// Open relay legs by hand for each PC and link, and run until the
    /// server joins them: the connections to the other ends, in order.
    fn join_by_hand(&mut self, links: Vec<(usize, LinkInfo)>) -> Vec<Connection> {
        let mut legs = Vec::new();
        for (i, link) in links {
            let (server_end, pc_end) = Connection::pair();
            self.w
                .server
                .accept(server_end, Route::Link, ip(i as u8 + 1), self.w.now);
            legs.push(self.w.pcs[i].open_leg(link, pc_end));
        }
        let mut ends: Vec<Option<Connection>> = legs.iter().map(|_| None).collect();
        while ends.iter().any(Option::is_none) {
            self.step();
            for (leg, end) in legs.iter_mut().zip(&mut ends) {
                if end.is_none() {
                    *end = leg.poll().unwrap();
                }
            }
        }
        ends.into_iter().flatten().collect()
    }
}

/// Whether `conn` is closed, once what's on its way has been read.
fn closed(conn: &mut Connection) -> bool {
    (0..1000).any(|_| conn.receive().is_err())
}

#[test]
fn leaders_search_cancel_and_quickmatch() {
    let mut a = Arena::new("search", 3, None);
    let party = |a: &Arena, i: usize| a.w.party(i).clone();
    // Alpha searches Double Team, and sees how it goes.
    a.send(0, ToServer::Search(DOUBLE_TEAM));
    a.until(5.0, |a| party(a, 0).activity == Activity::Searching);
    assert_eq!(party(&a, 0).playlist, DOUBLE_TEAM);
    a.until(5.0, |a| a.w.pcs[0].view.status.is_some());
    let status = a.w.pcs[0].view.status.unwrap();
    assert_eq!(
        (status.stage, status.have, status.need),
        (Stage::Searching, 1, 3)
    );
    // Everyone sees how many search it, within a few seconds.
    a.until(6.0, |a| {
        let playlists = &a.w.pcs[2].view.playlists;
        playlists
            .iter()
            .any(|p| p.id == DOUBLE_TEAM && p.searching == 1)
    });
    // Quickmatch picks the playlist most people search.
    a.send(1, ToServer::Search(QUICKMATCH));
    a.until(5.0, |a| party(a, 1).playlist == DOUBLE_TEAM);
    a.until(5.0, |a| {
        let status = a.w.pcs[0].view.status.unwrap();
        (status.stage, status.have) == (Stage::Gathering, 2)
    });
    // A party too big for a playlist is told why it can't search.
    a.w.pcs[2].send(ToServer::Guests(1));
    a.until(5.0, |a| party(a, 2).members[0].guests == 1);
    a.send(2, ToServer::Search(DOUBLE_TEAM));
    a.until(5.0, |a| !a.w.notices(2).is_empty());
    assert_eq!(
        a.w.notices(2),
        ["GUESTS ARE NOT ALLOWED IN RANKED MATCHMADE GAMES"]
    );
    assert_eq!(party(&a, 2).activity, Activity::Lobby);
    // Cancelling stops the search, and the other party is alone again.
    a.send(1, ToServer::Cancel);
    a.until(5.0, |a| party(a, 1).activity == Activity::Lobby);
    a.until(5.0, |a| {
        a.w.pcs[0].view.status.unwrap().stage == Stage::Searching
    });
    a.until(6.0, |a| {
        let playlists = &a.w.pcs[2].view.playlists;
        playlists
            .iter()
            .any(|p| p.id == DOUBLE_TEAM && p.searching == 1)
    });
    // Only leaders search: Charlie joins Alpha's party, which has to search
    // again since it changed.
    a.w.pcs[2].send(ToServer::Guests(0));
    a.send(2, ToServer::JoinParty(party(&a, 0).id));
    a.until(5.0, |a| party(a, 0).members.len() == 2);
    a.until(5.0, |a| party(a, 0).activity == Activity::Lobby);
    assert!(a
        .w
        .notices(0)
        .contains(&"YOUR PARTY CHANGED. SEARCH AGAIN.".to_string()));
    a.send(2, ToServer::Search(DOUBLE_TEAM));
    a.run(2.0);
    assert_eq!(party(&a, 0).activity, Activity::Lobby);
    // Everyone sees what the parties do.
    a.send(0, ToServer::Search(DOUBLE_TEAM));
    a.until(5.0, |a| {
        let online = &a.w.pcs[1].view.online;
        online
            .iter()
            .filter(|p| p.activity == Activity::Searching)
            .count()
            == 2
    });
}

/// How Double Team at levels 10, 12, 8 and 9 changes each player's XP and
/// level, by `levels`' tables: Alpha (10) and Delta (9) against Bravo (12)
/// and Charlie (8), the closest split. If Alpha and Delta win, Alpha beats
/// a 12 (115) and an 8 (85), +100; Delta beats a 12 (121) and an 8 (92),
/// +107 (106.5 rounded); Bravo loses to a 10 (-115) and a 9 (-121), times
/// level 12's 57.5%, -68; Charlie loses to a 10 (-85) and a 9 (-92), times
/// level 8's 27.5%, -24. Otherwise Bravo +82 ((85 + 79) / 2), Charlie +112
/// ((115 + 108) / 2), Alpha -40 (-100 at 40%) and Delta -33 (-93.5 at
/// 35%).
const LEVELS: [u8; 4] = [10, 12, 8, 9];
const ALPHA_DELTA_WIN: [(u32, u8); 4] = [(1000, 11), (1032, 11), (676, 8), (907, 10)];
const BRAVO_CHARLIE_WIN: [(u32, u8); 4] = [(860, 10), (1182, 12), (812, 9), (767, 9)];

#[test]
fn four_players_play_double_team_through_the_relay() {
    let mut a = Arena::new("double-team", 4, Some(("double_team", &LEVELS)));
    a.search(DOUBLE_TEAM);
    let m = a.formed();
    assert_eq!((m.playlist, m.ranked, m.bots), (DOUBLE_TEAM, true, 0));
    assert_eq!((m.game_type, m.score), (GameType::TeamSlayer, 5));
    assert!(["lockout", "midship"].contains(&m.map.as_str()));
    let team = |a: &Arena, i: usize| {
        m.players
            .iter()
            .find(|p| p.account == a.id(i))
            .unwrap()
            .team
    };
    assert_eq!(team(&a, 0), team(&a, 3));
    assert_eq!(team(&a, 1), team(&a, 2));
    assert_ne!(team(&a, 0), team(&a, 1));
    let levels: Vec<u8> = (0..4)
        .map(|i| {
            m.players
                .iter()
                .find(|p| p.account == a.id(i))
                .unwrap()
                .level
        })
        .collect();
    assert_eq!(levels, LEVELS);
    // Everyone's party is playing it.
    a.until(5.0, |a| {
        (0..4).all(|i| a.w.party(i).activity == Activity::Playing)
    });

    // The host links to everyone through the relay, and the game starts.
    let host = a.pc(m.host);
    a.until(30.0, |a| {
        a.gamers[host].hosted.as_ref().is_some_and(|h| h.started)
    });
    a.until(30.0, |a| {
        a.gamers[host]
            .hosted
            .as_ref()
            .is_some_and(|h| h.game.players.len() == 4)
    });
    let joined: Vec<usize> = (0..4).filter(|&i| i != host).collect();
    for &i in &joined {
        assert!(a.gamers[i].joined.is_some());
    }
    // Everyone plays as who the server says, on their team.
    let hosted = a.gamers[host].hosted.as_ref().unwrap();
    for p in &m.players {
        let spartan = hosted.game.players.iter().find(|s| s.name == p.gamertag);
        assert_eq!(spartan.map(|s| s.team), Some(p.team), "{}", p.gamertag);
    }
    // Playing to the end. The match is over as soon as every PC said how
    // it went.
    a.until(10.0, |a| a.w.server.relayed(m.id) > 50_000);
    a.until(120.0, |a| a.hosted(host).is_some_and(|h| h.game.over()));
    let ended = a.w.now;
    a.finish(&m);
    assert!(a.w.now - ended < 1.0);

    // Levels moved as the tables say, for whichever team won.
    let alpha_won = a.rank(0, "double_team").xp > min_xp(LEVELS[0]);
    let expected = if alpha_won {
        ALPHA_DELTA_WIN
    } else {
        BRAVO_CHARLIE_WIN
    };
    for (i, &(xp, level)) in expected.iter().enumerate() {
        assert_eq!(
            a.rank(i, "double_team"),
            Rank { xp, level },
            "{}",
            GAMERTAGS[i]
        );
        let over = a.over(i, m.id);
        assert!(over.counted);
        assert_eq!(over.reason, "");
        assert_eq!(over.levels, [(DOUBLE_TEAM, LEVELS[i], level)]);
        // The new stat card, kept for next time.
        assert_eq!(a.w.card(i as u8 + 1), over.card);
        let stats = a.account(i).stats("double_team").unwrap();
        let won = (i == 0 || i == 3) == alpha_won;
        assert_eq!((stats.games, stats.wins), (11, 5 + u32::from(won)));
    }
    // Each PC sees its new level, and its party is back in the lobby.
    a.until(5.0, |a| {
        (0..4).all(|i| a.w.party(i).activity == Activity::Lobby)
    });
    for (i, &(_, level)) in expected.iter().enumerate() {
        assert_eq!(a.w.welcome(i).levels, [(DOUBLE_TEAM, level, 11)]);
        let playlists = &a.w.pcs[i].view.playlists;
        let double_team = playlists.iter().find(|p| p.id == DOUBLE_TEAM).unwrap();
        assert_eq!(double_team.level, level);
    }
    // The match is logged, and the levels are on disk.
    let log = a.games_log();
    assert_eq!(log.lines().count(), 1);
    assert!(log.contains(&format!(" {} double_team {} team_slayer 1 ", m.id, m.map)));
    let ids: Vec<u64> = (0..4).map(|i| a.id(i)).collect();
    let accounts = a.w.accounts_txt();
    a.w.restart();
    assert_eq!(a.w.accounts_txt(), accounts);
    for (id, &(xp, level)) in ids.iter().zip(&expected) {
        let account = a.w.server.account(*id).unwrap();
        assert_eq!(
            account.stats("double_team").unwrap().rank,
            Rank { xp, level }
        );
    }
}

/// The XP and level a player at `level` (with the least XP for it) has
/// after a game worth `change`.
fn after(level: u8, change: i32) -> Rank {
    Rank {
        xp: min_xp(level),
        level,
    }
    .after(change)
}

/// What losing match `m` as its last player costs `loser` (an account), as
/// `levels` works it out: a loss to everyone on another side.
fn last_place_loss(m: &MatchInfo, loser: u64) -> i32 {
    let teams = m.game_type.teams();
    let game: Vec<Placed> = m
        .players
        .iter()
        .map(|p| Placed {
            level: p.level,
            team: p.team,
            place: u8::from(p.account == loser),
            bot: false,
        })
        .collect();
    let i = m.players.iter().position(|p| p.account == loser).unwrap();
    crate::levels::xp_changes(&game, teams)[i]
}

#[test]
fn a_host_that_quits_voids_the_match_and_loses_it() {
    let mut a = Arena::new("host-quits", 4, Some(("double_team", &LEVELS)));
    a.search(DOUBLE_TEAM);
    let m = a.formed();
    a.playing(&m, 4);
    let host = a.pc(m.host);
    a.quit(host);
    a.finish(&m);
    // It doesn't count for anyone else.
    for i in (0..4).filter(|&i| i != host) {
        let over = a.over(i, m.id);
        assert!(!over.counted);
        assert_eq!(over.reason, "THE HOST LEFT. THE GAME DIDN'T COUNT.");
        assert!(over.levels.is_empty());
        assert_eq!(a.rank(i, "double_team"), after(LEVELS[i], 0));
        assert_eq!(a.account(i).stats("double_team").unwrap().games, 10);
    }
    // The host comes last.
    let loss = last_place_loss(&m, m.host);
    assert!(loss < 0);
    let rank = after(LEVELS[host], loss);
    assert_eq!(a.rank(host, "double_team"), rank);
    let over = a.over(host, m.id);
    assert!(over.counted);
    assert_eq!(over.reason, "YOU QUIT AS HOST. IT COUNTS AS A LOSS.");
    assert_eq!(over.levels, [(DOUBLE_TEAM, LEVELS[host], rank.level)]);
    assert!(a.games_log().contains(&format!(" {} double_team ", m.id)));

    // It's asked to host last next time.
    a.until(5.0, |a| {
        (0..4).all(|i| a.w.party(i).activity == Activity::Lobby)
    });
    a.search(DOUBLE_TEAM);
    let next = a.formed();
    assert_ne!(next.id, m.id);
    assert_ne!(next.host, m.host);
    // Not on the map they just played, either.
    assert_ne!(next.map, m.map);
}

#[test]
fn a_host_whose_link_to_the_server_dies_loses_too() {
    let mut a = Arena::new("host-gone", 4, Some(("double_team", &LEVELS)));
    a.search(DOUBLE_TEAM);
    let m = a.formed();
    a.playing(&m, 4);
    let host = a.pc(m.host);
    // Its game goes on, but it stops answering the server.
    a.w.frozen.push(host);
    let joiners: Vec<usize> = (0..4).filter(|&i| i != host).collect();
    a.until(20.0, |a| {
        joiners.iter().all(|&i| !a.gamers[i].over.is_empty())
    });
    for &i in &joiners {
        assert_eq!(
            a.over(i, m.id).reason,
            "THE HOST LEFT. THE GAME DIDN'T COUNT."
        );
        assert_eq!(a.rank(i, "double_team"), after(LEVELS[i], 0));
    }
    let loss = last_place_loss(&m, m.host);
    let account = a.w.server.account(m.host).unwrap();
    let stats = account.stats("double_team").unwrap();
    assert_eq!(stats.rank, after(LEVELS[host], loss));
    assert_eq!((stats.games, stats.wins), (11, 5));
}

#[test]
fn a_player_who_quits_loses_and_comes_last() {
    // Rumble Pit, all at level 10.
    let mut a = Arena::new("joiner-quits", 3, Some(("ffa", &[10, 10, 10])));
    a.search(RUMBLE_PIT);
    let m = a.formed();
    assert_eq!((m.game_type, m.score), (GameType::Slayer, 3));
    a.playing(&m, 3);
    let quitter = (0..3).find(|&i| a.id(i) != m.host).unwrap();
    a.quit(quitter);
    // The game goes on without them (their Spartan with it), to its end.
    a.finish(&m);
    let over = a.over(quitter, m.id);
    assert!(over.counted);
    // Last whatever their score: a loss to each of the others at level
    // 10, -100 at 40%.
    assert_eq!(a.rank(quitter, "ffa"), after(10, -40));
    assert_eq!(over.levels, [(RUMBLE_PIT, 10, 10)]);
    // The two who stayed: a win over the quitter each, and first beat
    // second (+100 and 0) or they tied (+50 each).
    let mut stayed: Vec<u32> = (0..3)
        .filter(|&i| i != quitter)
        .map(|i| a.rank(i, "ffa").xp)
        .collect();
    stayed.sort_unstable();
    assert!(stayed == [900, 1000] || stayed == [950, 950], "{stayed:?}");
    let log = a.games_log();
    let quit = format!("{:016x}:0:2:1:900:860:10:10", a.id(quitter));
    assert!(log.contains(&quit), "{log}");
}

#[test]
fn results_that_disagree_with_the_hosts_void_the_match() {
    let mut a = Arena::new("disputed", 4, Some(("double_team", &LEVELS)));
    a.search(DOUBLE_TEAM);
    let m = a.formed();
    // Two of the three joined PCs say the other team won.
    let joiners: Vec<usize> = (0..4).filter(|&i| a.id(i) != m.host).collect();
    a.gamers[joiners[0]].lies = true;
    a.gamers[joiners[1]].lies = true;
    a.finish(&m);
    for (i, &level) in LEVELS.iter().enumerate() {
        let over = a.over(i, m.id);
        assert!(!over.counted);
        assert_eq!(
            over.reason,
            "THE RESULTS DIDN'T AGREE. THE GAME DIDN'T COUNT."
        );
        assert_eq!(a.rank(i, "double_team"), after(level, 0));
    }
    assert!(a
        .games_log()
        .contains(&format!(" {} double_team {} team_slayer 0 ", m.id, m.map)));
}

#[test]
fn big_team_battle_fills_up_with_bots_and_starts_without_a_pc_that_never_links() {
    let mut a = Arena::new("big-team", 4, None);
    a.search(BIG_TEAM);
    let m = a.formed();
    // Four people (more than the fewest): a countdown first, then bots
    // up to twelve.
    assert!(a.w.now >= 20.0);
    assert_eq!((m.playlist, m.ranked, m.bots), (BIG_TEAM, false, 8));
    assert_eq!(m.players.len(), 4);
    let reds = m.players.iter().filter(|p| p.team == 0).count();
    assert_eq!(reds, 2);
    // One PC never links: the game starts 20 seconds after hosting
    // without it.
    let lazy = (0..4).find(|&i| a.id(i) != m.host).unwrap();
    a.gamers[lazy].wont_link = true;
    let host = a.pc(m.host);
    a.until(10.0, |a| a.hosted(host).is_some());
    let hosting = a.w.now;
    a.until(30.0, |a| a.hosted(host).is_some_and(|h| h.started));
    assert!(a.w.now - hosting > 19.0);
    let over = a.over(lazy, m.id);
    assert_eq!(
        (over.counted, over.reason.as_str()),
        (false, "YOU DIDN'T JOIN THE GAME.")
    );
    a.until(5.0, |a| a.w.party(lazy).activity == Activity::Lobby);
    // Everyone sees three playing it.
    a.until(10.0, |a| {
        let playlists = &a.w.pcs[lazy].view.playlists;
        playlists.iter().any(|p| p.id == BIG_TEAM && p.playing == 3)
    });
    a.playing(&m, 3 + 8);
    a.finish(&m);
    // It doesn't change levels.
    for i in (0..4).filter(|&i| i != lazy) {
        let over = a.over(i, m.id);
        assert_eq!((over.counted, over.reason.as_str()), (false, ""));
        assert!(a.account(i).stats.is_empty());
    }
    let log = a.games_log();
    assert!(log.contains(&format!(" {} big_team {} team_slayer 0 ", m.id, m.map)));
}

#[test]
fn another_pc_hosts_if_the_first_asked_doesnt() {
    let mut a = Arena::new("next-host", 4, Some(("double_team", &LEVELS)));
    for g in &mut a.gamers {
        g.wont_host = true;
    }
    a.search(DOUBLE_TEAM);
    let m = a.formed();
    let first = a.pc(m.host);
    for (i, g) in a.gamers.iter_mut().enumerate() {
        g.wont_host = i == first;
    }
    // 45 seconds on, everyone hears of the next host, who hosts.
    let asked = a.w.now;
    a.until(50.0, |a| {
        a.gamers[0].info.as_ref().is_some_and(|i| i.host != m.host)
    });
    assert!(a.w.now - asked > 44.0);
    let next = a.gamers[0].info.clone().unwrap();
    assert_eq!(next.id, m.id);
    let host = a.pc(next.host);
    a.until(5.0, |a| a.hosted(host).is_some());
    // The first asked plays as anyone else, and it counts.
    a.finish(&next);
    assert!(a.gamers[first].over[0].counted);
    assert!((0..4).all(|i| a.over(i, m.id).counted));
}

#[test]
fn a_party_plays_custom_games_together() {
    let mut a = Arena::new("custom", 3, None);
    let party = a.w.party(0).id;
    for i in [1, 2] {
        a.send(i, ToServer::JoinParty(party));
    }
    a.until(5.0, |a| a.w.members(0).len() == 3);
    // Only the leader opens one.
    a.send(1, ToServer::Custom);
    a.run(1.0);
    assert!(a.gamers.iter().all(|g| g.custom.is_none()));
    a.send(0, ToServer::Custom);
    a.until(5.0, |a| a.gamers.iter().all(|g| g.custom.is_some()));
    let leader = a.id(0);
    assert!(a
        .gamers
        .iter()
        .all(|g| g.custom == Some((leader, "lockout".into()))));
    a.until(5.0, |a| a.w.party(1).activity == Activity::Custom);
    // Everyone else comes into the leader's lobby through the relay.
    a.until(5.0, |a| a.hosted(0).is_some_and(|h| h.host.joined() == 2));
    // The leader moves it to another map everyone has.
    a.send(0, ToServer::CustomMap("MIDSHIP".into()));
    a.until(5.0, |a| {
        a.gamers
            .iter()
            .all(|g| g.custom == Some((leader, "midship".into())))
    });
    // And starts a game, played to the end.
    a.gamers[0].hosted.as_mut().unwrap().start();
    a.until(10.0, |a| {
        let in_game = |i: usize| a.gamers[i].joined.as_ref().unwrap().client.in_game;
        in_game(1) && in_game(2)
    });
    assert_eq!(a.hosted(0).unwrap().game.players.len(), 3);
    a.until(120.0, |a| a.hosted(0).is_some_and(|h| h.game.over()));
    assert!(a.w.server.relayed(party) > 50_000);
    // Back in the lobby, someone who joins the party later comes in too.
    let h = a.gamers[0].hosted.as_mut().unwrap();
    h.host.set_lobby(lobby("midship"));
    h.started = false;
    let d = a.sign_in(4);
    a.send(d, ToServer::JoinParty(party));
    a.until(5.0, |a| {
        a.gamers[d].custom == Some((leader, "midship".into()))
    });
    a.until(5.0, |a| a.hosted(0).is_some_and(|h| h.host.joined() == 3));
    // When the leader leaves it, everyone is back in the party lobby.
    a.send(0, ToServer::Back);
    a.gamers[0].hosted = None;
    a.until(5.0, |a| {
        (0..4).all(|i| a.w.party(i).activity == Activity::Lobby)
    });
    a.until(5.0, |a| {
        (1..4).all(|i| a.gamers[i].joined.as_ref().unwrap().lost)
    });
    a.until(5.0, |a| a.w.server.connections() == 4);
    // Custom games change no one's levels.
    for i in 0..4 {
        assert!(a.account(i).stats.is_empty());
    }
    assert_eq!(a.games_log(), "");
}

#[test]
fn the_relay_drops_a_link_that_falls_behind_and_legs_left_waiting() {
    let mut a = Arena::new("backpressure", 2, None);
    let party = a.w.party(0).id;
    a.send(1, ToServer::JoinParty(party));
    a.until(5.0, |a| a.w.members(0).len() == 2);
    // The legs are opened by hand.
    for g in &mut a.gamers {
        g.wont_link = true;
    }
    a.send(0, ToServer::Custom);
    let link = |a: &Arena, i: usize| {
        a.w.events[i].iter().rev().find_map(|e| match e {
            LiveEvent::Link(link) => Some(link.clone()),
            _ => None,
        })
    };
    a.until(5.0, |a| link(a, 0).is_some() && link(a, 1).is_some());
    let (host_link, joiner_link) = (link(&a, 0).unwrap(), link(&a, 1).unwrap());
    assert_ne!(host_link.token, joiner_link.token);
    assert_eq!((host_link.joiner, joiner_link.joiner), (false, true));
    assert_eq!((host_link.peer, joiner_link.peer), (a.id(1), a.id(0)));
    let mut legs = Vec::new();
    for (i, link) in [(0, host_link), (1, joiner_link)] {
        let (server_end, pc_end) = Connection::pair();
        a.w.server
            .accept(server_end, Route::Link, ip(i as u8 + 1), a.w.now);
        legs.push(a.w.pcs[i].open_leg(link, pc_end));
    }
    let mut ends: Vec<Connection> = Vec::new();
    while ends.len() < 2 {
        a.step();
        for leg in &mut legs {
            if let Some(conn) = leg.poll().unwrap() {
                ends.push(conn);
            }
        }
    }
    // Messages go through as they are, either way.
    let (mut host, mut joiner) = (ends.remove(0), ends.remove(0));
    host.send(7, b"from the host");
    joiner.send(99, &[]);
    host.flush().unwrap();
    joiner.flush().unwrap();
    a.step();
    assert_eq!(joiner.receive().unwrap(), [(7, b"from the host".to_vec())]);
    assert_eq!(host.receive().unwrap(), [(99, Vec::new())]);
    let relayed = a.w.server.relayed(party);
    assert_eq!(relayed, 15);
    // The joined PC stops reading, and the host keeps sending: once more
    // than 2 MB waits for it, the link is dropped.
    let big = vec![0; 100_000];
    let mut sent = 0;
    while a.w.server.connections() > 2 {
        assert!(sent < 10_000_000, "never dropped");
        for _ in 0..5 {
            host.send(1, &big);
            sent += big.len() + 1;
        }
        host.flush().unwrap();
        a.step();
    }
    assert!(a.w.server.relayed(party) > 2_000_000 + relayed);
    // Both ends find the link gone.
    a.run(3.0);
    assert!(host.receive().is_err());
    let mut lost = false;
    for _ in 0..1000 {
        if joiner.receive().is_err() {
            lost = true;
            break;
        }
    }
    assert!(lost);

    // A leg whose other end never comes is closed after 20 seconds.
    a.send(1, ToServer::LeaveParty);
    a.until(5.0, |a| a.w.members(0).len() == 1);
    a.send(1, ToServer::JoinParty(party));
    let seen = a.w.events[0].len();
    a.until(5.0, |a| {
        a.w.events[0][seen..]
            .iter()
            .any(|e| matches!(e, LiveEvent::Link(_)))
    });
    let link = link(&a, 0).unwrap();
    let (server_end, pc_end) = Connection::pair();
    a.w.server.accept(server_end, Route::Link, ip(1), a.w.now);
    let mut leg = a.w.pcs[0].open_leg(link, pc_end);
    a.run(19.0);
    assert!(matches!(leg.poll(), Ok(None)));
    a.run(2.0);
    assert!(leg.poll().is_err());
}

#[test]
fn a_host_cant_stand_in_for_the_pcs_joining_it() {
    let mut a = Arena::new("stand-in", 2, None);
    let party = a.w.party(0).id;
    a.send(1, ToServer::JoinParty(party));
    a.until(5.0, |a| a.w.members(0).len() == 2);
    for g in &mut a.gamers {
        g.wont_link = true;
    }
    a.send(0, ToServer::Custom);
    let (leader, member) = (a.id(0), a.id(1));
    a.until(5.0, |a| {
        a.link(0, member).is_some() && a.link(1, leader).is_some()
    });
    let (host_link, joiner_link) = (a.link(0, member).unwrap(), a.link(1, leader).unwrap());
    // Each end has a token of its own: the host can't say it's the other.
    assert_ne!(host_link.token, joiner_link.token);
    let (server_end, mut fake) = Connection::pair();
    a.w.server.accept(server_end, Route::Link, ip(1), a.w.now);
    let hello = ToServer::LinkHello {
        token: host_link.token,
        account: member,
    };
    hello.send(&mut fake);
    fake.flush().unwrap();
    a.step();
    assert!(closed(&mut fake));
    // The real legs are joined.
    let ends = a.join_by_hand(vec![(0, host_link), (1, joiner_link)]);
    assert_eq!(ends.len(), 2);
}

#[test]
fn links_close_with_the_custom_game_they_were_for() {
    let mut a = Arena::new("links-close", 3, None);
    let party = a.w.party(0).id;
    for i in [1, 2] {
        a.send(i, ToServer::JoinParty(party));
    }
    a.until(5.0, |a| a.w.members(0).len() == 3);
    for g in &mut a.gamers {
        g.wont_link = true;
    }
    a.send(0, ToServer::Custom);
    let ids = [a.id(0), a.id(1), a.id(2)];
    a.until(5.0, |a| {
        (1..3).all(|i| a.link(0, ids[i]).is_some() && a.link(i, ids[0]).is_some())
    });
    let links = vec![
        (0, a.link(0, ids[1]).unwrap()),
        (1, a.link(1, ids[0]).unwrap()),
        (0, a.link(0, ids[2]).unwrap()),
        (2, a.link(2, ids[0]).unwrap()),
    ];
    let [mut to_bravo, mut bravo, mut to_charlie, mut charlie] =
        <[Connection; 4]>::try_from(a.join_by_hand(links))
            .ok()
            .unwrap();
    // The leader removes Charlie: Charlie's link closes, Bravo's stays.
    a.send(0, ToServer::Kick(ids[2]));
    a.run(3.0);
    assert!(closed(&mut charlie));
    assert!(closed(&mut to_charlie));
    to_bravo.send(7, b"still here");
    to_bravo.flush().unwrap();
    a.step();
    assert_eq!(bravo.receive().unwrap(), [(7, b"still here".to_vec())]);
    // The leader goes back to the party lobby: the game is over, and so is
    // Bravo's link.
    a.send(0, ToServer::Back);
    a.run(3.0);
    assert!(closed(&mut bravo));
    assert!(closed(&mut to_bravo));
    assert_eq!(a.w.server.connections(), 3);
}
