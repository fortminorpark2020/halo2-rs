use super::*;
use h2sim::game::{Emblem, Event, Look};
use h2sim::testing::{floor, game};
use h2sim::Command;
use std::collections::VecDeque;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

/// Poll both ends until `done` or a few seconds pass.
fn pump(
    host: &mut Host,
    host_game: &mut h2sim::Game,
    client: &mut Client,
    client_game: &mut h2sim::Game,
    mut done: impl FnMut(&[HostEvent], &[ClientEvent], &h2sim::Game) -> bool,
) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        let he = host.poll(host_game, 16);
        host.send(host_game, &[], false);
        let (ce, _) = client.poll(client_game);
        if done(&he, &ce, client_game) {
            return;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("timed out");
}

fn address(host: &Host) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, host.port()))
}

fn me() -> (&'static str, Look) {
    ("TESTER", Look::default())
}

#[test]
fn a_joined_pc_plays_in_the_hosts_game() {
    let world = floor();
    let mut hg = game();
    let me = hg.add_player();
    let mut host = Host::new("testmap", 1).unwrap();
    let mut cg = game();
    let look = Look {
        elite: true,
        colors: [12, 3],
        emblem: Emblem {
            foreground: 63,
            background: 31,
            colors: [17, 0, 9],
        },
    };
    let mut client = Client::connect(
        address(&host),
        &cg,
        "testmap",
        &[ANY_TEAM],
        ("Noble Six", look),
    )
    .unwrap();

    let mut mine = None;
    pump(&mut host, &mut hg, &mut client, &mut cg, |he, ce, cg| {
        if let Some(HostEvent::Joined { players, .. }) = he.first() {
            assert_eq!(players.len(), 1);
        }
        for e in ce {
            match e {
                ClientEvent::Welcomed { players, .. } => mine = Some(players[0]),
                other => panic!("{other:?}"),
            }
        }
        mine.is_some() && cg.players.len() == 2
    });
    let mine = mine.unwrap();
    assert_ne!(mine, me);
    assert!(host.is_remote(mine));
    // They go by the name and look of the person at that PC, on both PCs.
    assert_eq!(hg.players[mine].name, "NOBLE SIX");
    assert_eq!(cg.players[mine].name, "NOBLE SIX");
    assert_eq!(hg.players[mine].look, look);
    assert_eq!(cg.players[mine].look, look);

    // The joined player walks forward and fires; the host runs it and the
    // joined PC sees the result.
    let start = hg.players[mine].body.position;
    let walk = Command {
        movement: glam::Vec2::new(0.0, 1.0),
        yaw: 0.0,
        fire: true,
        ..Command::default()
    };
    let mut shots = 0;
    for _ in 0..40 {
        client.send_commands(&[(mine, walk)]);
        std::thread::sleep(Duration::from_millis(2));
        host.poll(&mut hg, 16);
        let mut commands = vec![Command::default(); hg.players.len()];
        commands[mine] = host.command(mine).unwrap();
        hg.step(&world, &commands);
        let events = std::mem::take(&mut hg.events);
        host.send(&hg, &events, true);
        std::thread::sleep(Duration::from_millis(2));
        let (ce, events) = client.poll(&mut cg);
        assert!(ce.is_empty(), "{ce:?}");
        shots += events
            .iter()
            .filter(|e| matches!(e, Event::Shot { player, .. } if *player == mine))
            .count();
    }
    assert!(hg.players[mine].body.position.x > start.x + 0.3);
    assert!(shots > 0);
    // Give the last snapshot time to land.
    pump(&mut host, &mut hg, &mut client, &mut cg, |_, _, _| true);
    let joined = cg.players[mine].body.position;
    assert!(joined.x > start.x + 0.3, "{joined} vs {start}");

    // Leaving hands the Spartan back to the host.
    drop(client);
    let start = Instant::now();
    loop {
        let events = host.poll(&mut hg, 16);
        if let Some(HostEvent::Left { players, .. }) = events.first() {
            assert_eq!(players, &vec![mine]);
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!host.is_remote(mine));
}

#[test]
fn taps_between_ticks_are_not_lost() {
    let mut hg = game();
    hg.add_player();
    let mut host = Host::new("testmap", 2).unwrap();
    let mut cg = game();
    let mut client = Client::connect(address(&host), &cg, "testmap", &[ANY_TEAM], me()).unwrap();
    let mut mine = None;
    pump(&mut host, &mut hg, &mut client, &mut cg, |_, ce, _| {
        if let Some(ClientEvent::Welcomed { players, .. }) = ce.first() {
            mine = Some(players[0]);
        }
        mine.is_some()
    });
    let mine = mine.unwrap();
    let tap = Command {
        melee: true,
        ..Command::default()
    };
    client.send_commands(&[(mine, tap)]);
    client.send_commands(&[(mine, Command::default())]);
    let start = Instant::now();
    while host.command(mine) == Some(Command::default()) {
        host.poll(&mut hg, 16);
        assert!(start.elapsed() < Duration::from_secs(5));
    }
    // Pressed for one tick, then released.
    assert_eq!(host.command(mine), Some(Command::default()));
}

#[test]
fn a_pc_on_another_map_is_turned_away() {
    let mut hg = game();
    hg.add_player();
    let mut host = Host::new("lockout", 3).unwrap();
    let mut cg = game();
    let mut client = Client::connect(address(&host), &cg, "midship", &[ANY_TEAM], me()).unwrap();
    let mut refused = None;
    pump(&mut host, &mut hg, &mut client, &mut cg, |_, ce, _| {
        if let Some(ClientEvent::Refused(why)) = ce.first() {
            refused = Some(why.clone());
        }
        refused.is_some()
    });
    assert_eq!(refused.as_deref(), Some("HOST IS PLAYING LOCKOUT"));
    assert_eq!(hg.players.len(), 1);
}

#[test]
fn hosted_games_can_be_found() {
    let mut browser = Browser::new(10);
    let mut hg = game();
    let mut host = Host::new("lockout", 11).unwrap();
    let start = Instant::now();
    loop {
        host.poll(&mut hg, 16);
        if let Some(g) = browser
            .poll()
            .iter()
            .find(|g| g.address.port() == host.port())
        {
            assert_eq!(g.map, "lockout");
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5), "no beacon heard");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn joined_pcs_wait_in_the_lobby_between_games() {
    let mut hg = game();
    hg.add_player();
    let mut host = Host::new("lockout", 12).unwrap();
    let lobby = Lobby {
        map: "lockout".into(),
        game_type: "SLAYER".into(),
        score: "25".into(),
        options: "DEFAULT".into(),
        teams: false,
        players: vec![LobbyPlayer {
            name: "HOST".into(),
            look: Look::default(),
            team: 0,
            remote: false,
        }],
        bots: 2,
    };
    host.set_lobby(lobby.clone());
    // Any map will do for the lobby.
    let mut cg = game();
    let mut client = Client::connect(address(&host), &cg, "midship", &[ANY_TEAM, 1], me()).unwrap();
    let mut arrived = false;
    let mut seen = None;
    pump(&mut host, &mut hg, &mut client, &mut cg, |he, ce, _| {
        arrived |= he.iter().any(|e| matches!(e, HostEvent::Arrived { .. }));
        for e in ce {
            match e {
                ClientEvent::Lobby(l) => seen = Some(l.clone()),
                other => panic!("{other:?}"),
            }
        }
        arrived && seen.is_some()
    });
    assert_eq!(seen.as_ref(), Some(&lobby));
    assert_eq!(
        host.members(),
        vec![("TESTER".to_string(), Look::default(), vec![ANY_TEAM, 1])]
    );
    assert_eq!(hg.players.len(), 1);

    // Changes reach them.
    let fewer = Lobby { bots: 1, ..lobby };
    host.set_lobby(fewer.clone());
    let mut seen = None;
    pump(&mut host, &mut hg, &mut client, &mut cg, |_, ce, _| {
        if let Some(ClientEvent::Lobby(l)) = ce.first() {
            seen = Some(l.clone());
        }
        seen.is_some()
    });
    assert_eq!(seen, Some(fewer.clone()));

    // The host starts a team game: they load the map and join it, on the
    // teams they picked.
    hg.rules.game_type = h2sim::GameType::TeamSlayer;
    host.start("lockout");
    let mut started = None;
    pump(&mut host, &mut hg, &mut client, &mut cg, |_, ce, _| {
        if let Some(ClientEvent::Start(map)) = ce.first() {
            started = Some(map.clone());
        }
        started.is_some()
    });
    assert_eq!(started.as_deref(), Some("lockout"));
    client.rejoin(&cg, "lockout", &[1, 1]);
    let mut mine = Vec::new();
    pump(&mut host, &mut hg, &mut client, &mut cg, |_, ce, cg| {
        if let Some(ClientEvent::Welcomed { players, .. }) = ce.first() {
            mine = players.clone();
        }
        !mine.is_empty() && cg.players.len() == 3
    });
    assert_eq!(mine.len(), 2);
    assert!(mine.iter().all(|&p| hg.players[p].team == 1));
    assert!(client.in_game);
    assert!(host.is_remote(mine[0]));

    // Back to the lobby: their players are gone with the game.
    host.set_lobby(fewer);
    let mut back = false;
    pump(&mut host, &mut hg, &mut client, &mut cg, |_, ce, _| {
        back |= ce.iter().any(|e| matches!(e, ClientEvent::Lobby(_)));
        back
    });
    assert!(!client.in_game);
    assert!(!host.is_remote(mine[0]));
}

// The same over connections in memory and over WebSockets, as online games
// join through the service (no ports of their own), and a host on any free
// port.

/// The two ends of a WebSocket over loopback: the end a server took (a
/// host's, say, with the service in between), and the end that dialed it.
fn ws_pair() -> (Connection, Connection) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/link", listener.local_addr().unwrap());
    let dialing = dial(&url, Duration::from_secs(5));
    let (stream, _) = listener.accept().unwrap();
    let Ok(Request::WebSocket(taken, path)) = accept(stream) else {
        panic!("no WebSocket");
    };
    assert_eq!(path, "/link");
    (taken, dialing.recv().unwrap().unwrap())
}

/// An online host and a PC joining it, verified as `who`.
fn online(
    map: &str,
    cg: &h2sim::Game,
    client_map: &str,
    teams: &[u8],
    me: (&str, Look),
    who: Verified,
) -> (Host, Client) {
    online_over(Connection::pair(), map, cg, client_map, teams, me, who)
}

/// The same over `pair`: the host's end, then the PC's.
fn online_over(
    (a, b): (Connection, Connection),
    map: &str,
    cg: &h2sim::Game,
    client_map: &str,
    teams: &[u8],
    me: (&str, Look),
    who: Verified,
) -> (Host, Client) {
    let mut host = Host::online(map);
    host.add_connection(a, who);
    let client = Client::over(b, cg, client_map, teams, me);
    (host, client)
}

fn verified(gamertag: &str, team: u8) -> Verified {
    Verified {
        account: 7,
        gamertag: gamertag.into(),
        level: 12,
        team,
    }
}

#[test]
fn a_pc_joined_online_plays_in_the_hosts_game() {
    plays_in_the_hosts_game(Connection::pair());
}

#[test]
fn a_pc_joined_by_websocket_plays_in_the_hosts_game() {
    plays_in_the_hosts_game(ws_pair());
}

fn plays_in_the_hosts_game(pair: (Connection, Connection)) {
    let world = floor();
    let mut hg = game();
    let me = hg.add_player();
    let mut cg = game();
    let look = Look {
        elite: true,
        colors: [12, 3],
        emblem: Emblem {
            foreground: 63,
            background: 31,
            colors: [17, 0, 9],
        },
    };
    let (mut host, mut client) = online_over(
        pair,
        "testmap",
        &cg,
        "testmap",
        &[ANY_TEAM],
        ("Noble Six", look),
        verified("SPARTAN B312", ANY_TEAM),
    );
    assert_eq!(host.port(), 0);

    let mut mine = None;
    pump(&mut host, &mut hg, &mut client, &mut cg, |he, ce, cg| {
        if let Some(HostEvent::Joined { players, .. }) = he.first() {
            assert_eq!(players.len(), 1);
        }
        for e in ce {
            match e {
                ClientEvent::Welcomed { players, .. } => mine = Some(players[0]),
                other => panic!("{other:?}"),
            }
        }
        mine.is_some() && cg.players.len() == 2
    });
    let mine = mine.unwrap();
    assert_ne!(mine, me);
    assert!(host.is_remote(mine));
    // They go by the gamertag the service checked (not the one they said),
    // and their own look, on both PCs.
    assert_eq!(hg.players[mine].name, "SPARTAN B312");
    assert_eq!(cg.players[mine].name, "SPARTAN B312");
    assert_eq!(hg.players[mine].look, look);
    assert_eq!(cg.players[mine].look, look);

    let start = hg.players[mine].body.position;
    let walk = Command {
        movement: glam::Vec2::new(0.0, 1.0),
        yaw: 0.0,
        fire: true,
        ..Command::default()
    };
    let mut shots = 0;
    for _ in 0..40 {
        client.send_commands(&[(mine, walk)]);
        host.poll(&mut hg, 16);
        let mut commands = vec![Command::default(); hg.players.len()];
        commands[mine] = host.command(mine).unwrap();
        hg.step(&world, &commands);
        let events = std::mem::take(&mut hg.events);
        host.send(&hg, &events, true);
        let (ce, events) = client.poll(&mut cg);
        assert!(ce.is_empty(), "{ce:?}");
        shots += events
            .iter()
            .filter(|e| matches!(e, Event::Shot { player, .. } if *player == mine))
            .count();
        // Ticks a tick apart: online, the game goes out every other one.
        std::thread::sleep(Duration::from_secs_f32(h2sim::game::TICK));
    }
    assert!(hg.players[mine].body.position.x > start.x + 0.3);
    assert!(shots > 0);
    // The last tick goes with the next snapshot.
    std::thread::sleep(Duration::from_millis(40));
    let there = hg.players[mine].body.position;
    pump(&mut host, &mut hg, &mut client, &mut cg, |_, _, cg| {
        cg.players[mine].body.position == there
    });

    // Leaving hands the Spartan back to the host.
    drop(client);
    let start = Instant::now();
    let players = loop {
        if let Some(HostEvent::Left { players, .. }) = host.poll(&mut hg, 16).first() {
            break players.clone();
        }
        assert!(start.elapsed() < Duration::from_secs(5), "still there");
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(players, vec![mine]);
    assert!(!host.is_remote(mine));
}

#[test]
fn online_taps_between_ticks_are_not_lost() {
    taps_between_ticks_are_not_lost_over(Connection::pair());
}

#[test]
fn websocket_taps_between_ticks_are_not_lost() {
    taps_between_ticks_are_not_lost_over(ws_pair());
}

fn taps_between_ticks_are_not_lost_over(pair: (Connection, Connection)) {
    let mut hg = game();
    hg.add_player();
    let mut cg = game();
    let who = verified("TESTER", ANY_TEAM);
    let (mut host, mut client) =
        online_over(pair, "testmap", &cg, "testmap", &[ANY_TEAM], me(), who);
    let mut mine = None;
    pump(&mut host, &mut hg, &mut client, &mut cg, |_, ce, _| {
        if let Some(ClientEvent::Welcomed { players, .. }) = ce.first() {
            mine = Some(players[0]);
        }
        mine.is_some()
    });
    let mine = mine.unwrap();
    let tap = Command {
        melee: true,
        ..Command::default()
    };
    client.send_commands(&[(mine, tap)]);
    client.send_commands(&[(mine, Command::default())]);
    // Once they've arrived (at once, in memory).
    std::thread::sleep(Duration::from_millis(20));
    host.poll(&mut hg, 16);
    // Pressed for one tick, then released.
    assert_eq!(host.command(mine), Some(tap));
    assert_eq!(host.command(mine), Some(Command::default()));
}

#[test]
fn an_online_pc_on_another_map_is_turned_away() {
    a_pc_on_another_map_is_turned_away_over(Connection::pair());
}

#[test]
fn a_websocket_pc_on_another_map_is_turned_away() {
    a_pc_on_another_map_is_turned_away_over(ws_pair());
}

fn a_pc_on_another_map_is_turned_away_over(pair: (Connection, Connection)) {
    let mut hg = game();
    hg.add_player();
    let mut cg = game();
    let who = verified("TESTER", ANY_TEAM);
    let (mut host, mut client) =
        online_over(pair, "lockout", &cg, "midship", &[ANY_TEAM], me(), who);
    let mut refused = None;
    pump(&mut host, &mut hg, &mut client, &mut cg, |_, ce, _| {
        if let Some(ClientEvent::Refused(why)) = ce.first() {
            refused = Some(why.clone());
        }
        refused.is_some()
    });
    assert_eq!(refused.as_deref(), Some("HOST IS PLAYING LOCKOUT"));
    assert_eq!(hg.players.len(), 1);
}

#[test]
fn games_on_any_port_can_be_found_and_joined() {
    let mut browser = Browser::new(20);
    let mut hg = game();
    hg.add_player();
    // Every address, IPv4 too, on any free port.
    let any = SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, 0));
    let mut host = Host::bind("lockout", 21, any, true).unwrap();
    assert_ne!(host.port(), 0);
    let start = Instant::now();
    let found = loop {
        host.poll(&mut hg, 16);
        if let Some(g) = browser
            .poll()
            .iter()
            .find(|g| g.address.port() == host.port())
        {
            break g.clone();
        }
        assert!(start.elapsed() < Duration::from_secs(5), "no beacon heard");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(found.map, "lockout");
    let mut cg = game();
    let mut client = Client::connect(address(&host), &cg, "lockout", &[ANY_TEAM], me()).unwrap();
    let mut welcomed = false;
    pump(&mut host, &mut hg, &mut client, &mut cg, |_, ce, _| {
        welcomed |= matches!(ce.first(), Some(ClientEvent::Welcomed { .. }));
        welcomed
    });
}

#[test]
fn online_pcs_wait_in_the_lobby_between_games() {
    pcs_wait_in_the_lobby_between_games_over(Connection::pair());
}

#[test]
fn websocket_pcs_wait_in_the_lobby_between_games() {
    pcs_wait_in_the_lobby_between_games_over(ws_pair());
}

fn pcs_wait_in_the_lobby_between_games_over((a, b): (Connection, Connection)) {
    let mut hg = game();
    hg.add_player();
    let mut host = Host::online("lockout");
    let lobby = Lobby {
        map: "lockout".into(),
        game_type: "SLAYER".into(),
        score: "25".into(),
        options: "DEFAULT".into(),
        teams: false,
        players: vec![LobbyPlayer {
            name: "HOST".into(),
            look: Look::default(),
            team: 0,
            remote: false,
        }],
        bots: 2,
    };
    host.set_lobby(lobby.clone());
    // Any map will do for the lobby. The service put them on team 0,
    // whatever they'd like.
    let mut cg = game();
    host.add_connection(a, verified("BLUE TEAM", 0));
    let mut client = Client::over(b, &cg, "midship", &[ANY_TEAM, 1], me());
    let mut arrived = false;
    let mut seen = None;
    pump(&mut host, &mut hg, &mut client, &mut cg, |he, ce, _| {
        arrived |= he.iter().any(|e| matches!(e, HostEvent::Arrived { .. }));
        for e in ce {
            match e {
                ClientEvent::Lobby(l) => seen = Some(l.clone()),
                other => panic!("{other:?}"),
            }
        }
        arrived && seen.is_some()
    });
    assert_eq!(seen.as_ref(), Some(&lobby));
    assert_eq!(
        host.members(),
        vec![("BLUE TEAM".to_string(), Look::default(), vec![0, 0])]
    );
    assert_eq!(hg.players.len(), 1);

    // Changes reach them.
    let fewer = Lobby { bots: 1, ..lobby };
    host.set_lobby(fewer.clone());
    let mut seen = None;
    pump(&mut host, &mut hg, &mut client, &mut cg, |_, ce, _| {
        if let Some(ClientEvent::Lobby(l)) = ce.first() {
            seen = Some(l.clone());
        }
        seen.is_some()
    });
    assert_eq!(seen, Some(fewer.clone()));

    // The host starts a team game: they load the map and join it, on the
    // team the service gave them.
    hg.rules.game_type = h2sim::GameType::TeamSlayer;
    host.start("lockout");
    let mut started = None;
    pump(&mut host, &mut hg, &mut client, &mut cg, |_, ce, _| {
        if let Some(ClientEvent::Start(map)) = ce.first() {
            started = Some(map.clone());
        }
        started.is_some()
    });
    assert_eq!(started.as_deref(), Some("lockout"));
    client.rejoin(&cg, "lockout", &[1, 1]);
    let mut mine = Vec::new();
    pump(&mut host, &mut hg, &mut client, &mut cg, |_, ce, cg| {
        if let Some(ClientEvent::Welcomed { players, .. }) = ce.first() {
            mine = players.clone();
        }
        !mine.is_empty() && cg.players.len() == 3
    });
    assert_eq!(mine.len(), 2);
    assert!(mine.iter().all(|&p| hg.players[p].team == 0));
    assert_eq!(hg.players[mine[1]].name, "BLUE TEAM(1)");
    assert!(client.in_game);
    assert!(host.is_remote(mine[0]));

    // Back to the lobby: their players are gone with the game.
    host.set_lobby(fewer);
    let mut back = false;
    pump(&mut host, &mut hg, &mut client, &mut cg, |_, ce, _| {
        back |= ce.iter().any(|e| matches!(e, ClientEvent::Lobby(_)));
        back
    });
    assert!(!client.in_game);
    assert!(!host.is_remote(mine[0]));
}

#[test]
fn online_pcs_are_who_the_service_says() {
    // A team game with the host on one team and two on the other.
    let mut hg = game();
    hg.rules.game_type = h2sim::GameType::TeamSlayer;
    hg.add_player_on(0);
    hg.add_player_on(1);
    hg.add_player_on(1);
    let mut cg = game();
    let who = verified("RED", 1);
    let (mut host, mut client) = online("testmap", &cg, "testmap", &[ANY_TEAM], me(), who);
    let mut joined = None;
    let mut mine = None;
    pump(&mut host, &mut hg, &mut client, &mut cg, |he, ce, _| {
        if let Some(HostEvent::Joined { computer, .. }) = he.first() {
            joined = Some(computer.clone());
        }
        if let Some(ClientEvent::Welcomed { players, .. }) = ce.first() {
            mine = Some(players[0]);
        }
        joined.is_some() && mine.is_some()
    });
    // Known by their gamertag, not whatever their PC is called.
    assert_eq!(joined.as_deref(), Some("RED"));
    assert_eq!(hg.players[mine.unwrap()].team, 1);
    // Someone else there starts playing: on the same team, though the
    // other is smaller.
    client.add_local();
    let mut added = None;
    pump(&mut host, &mut hg, &mut client, &mut cg, |_, ce, _| {
        if let Some(ClientEvent::Added(p)) = ce.first() {
            added = Some(*p);
        }
        added.is_some()
    });
    let added = added.unwrap();
    assert_eq!(hg.players[added].team, 1);
    assert_eq!(hg.players[added].name, "RED(1)");
}

// Heartbeats, with a short timeout instead of the real ten seconds.

const SHORT: Duration = Duration::from_millis(300);

#[test]
fn a_silent_host_is_lost_within_the_timeout() {
    let mut cg = game();
    // From when the connection is made, as the timeout runs.
    let start = Instant::now();
    let (mut silent, end) = Connection::pair();
    let mut client = Client::over(end, &cg, "testmap", &[ANY_TEAM], me());
    client.set_timeout(SHORT);
    let why = loop {
        let (ce, _) = client.poll(&mut cg);
        if let Some(ClientEvent::Lost(why)) = ce.first() {
            break why.clone();
        }
        assert!(start.elapsed() < SHORT * 3, "still waiting");
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(why, "timed out");
    assert!(start.elapsed() >= SHORT);
    // Meanwhile it said hello, then that it was still there.
    let said: Vec<u8> = silent.receive().unwrap().iter().map(|m| m.0).collect();
    assert_eq!(said[0], kind::HELLO);
    assert!(said[1..].len() >= 5 && said[1..].iter().all(|&k| k == kind::ALIVE));
}

#[test]
fn a_silent_pc_is_dropped_within_the_timeout() {
    let mut hg = game();
    hg.add_player();
    let mut cg = game();
    let who = verified("TESTER", ANY_TEAM);
    let (mut host, mut client) = online("testmap", &cg, "testmap", &[ANY_TEAM], me(), who);
    host.set_timeout(SHORT);
    let mut mine = None;
    pump(&mut host, &mut hg, &mut client, &mut cg, |_, ce, _| {
        if let Some(ClientEvent::Welcomed { players, .. }) = ce.first() {
            mine = Some(players[0]);
        }
        mine.is_some()
    });
    // The PC stops sending anything (but stays connected).
    let start = Instant::now();
    let (players, reason) = loop {
        if let Some(HostEvent::Left {
            players, reason, ..
        }) = host.poll(&mut hg, 16).first()
        {
            break (players.clone(), reason.clone());
        }
        assert!(start.elapsed() < SHORT * 3, "still there");
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(players, vec![mine.unwrap()]);
    assert_eq!(reason, "timed out");
    assert!(start.elapsed() >= SHORT / 2);
}

#[test]
fn an_idle_lobby_stays_connected() {
    let mut hg = game();
    hg.add_player();
    let mut host = Host::bind("lockout", 30, "127.0.0.1:0".parse().unwrap(), false).unwrap();
    host.set_timeout(SHORT);
    host.set_lobby(Lobby::default());
    let mut cg = game();
    let mut client = Client::connect(address(&host), &cg, "midship", &[ANY_TEAM], me()).unwrap();
    client.set_timeout(SHORT);
    // Ten timeouts with nothing to say but that they're still there.
    let start = Instant::now();
    while start.elapsed() < SHORT * 10 {
        for e in host.poll(&mut hg, 16) {
            assert!(matches!(e, HostEvent::Arrived { .. }), "{e:?}");
        }
        let (ce, _) = client.poll(&mut cg);
        assert!(
            ce.iter().all(|e| matches!(e, ClientEvent::Lobby(_))),
            "{ce:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(host.joined(), 1);
}

#[test]
fn heartbeats_go_out_while_the_game_cannot_run() {
    let mut hg = game();
    let mut cg = game();
    let who = verified("TESTER", ANY_TEAM);
    let (mut host, mut client) = online("lockout", &cg, "lockout", &[ANY_TEAM], me(), who);
    host.set_timeout(SHORT);
    client.set_timeout(SHORT);
    host.set_lobby(Lobby::default());
    pump(&mut host, &mut hg, &mut client, &mut cg, |_, ce, _| {
        matches!(ce.first(), Some(ClientEvent::Lobby(_)))
    });
    // The host's window is minimized for a while: it can't read, but it
    // still says it's there.
    let start = Instant::now();
    while start.elapsed() < SHORT * 4 {
        host.keep_alive();
        let (ce, _) = client.poll(&mut cg);
        assert!(ce.is_empty(), "{ce:?}");
        std::thread::sleep(Duration::from_millis(5));
    }
    // And the joined PC's ones, read late, still count.
    assert!(host.poll(&mut hg, 16).is_empty());
    assert_eq!(host.joined(), 1);
}

#[test]
fn joined_pcs_send_their_controls_once_a_tick() {
    let cg = game();
    let (mut host_end, end) = Connection::pair();
    let mut client = Client::over(end, &cg, "testmap", &[ANY_TEAM], me());
    let inputs = |c: &mut Connection| {
        let got = c.receive().unwrap();
        got.iter().filter(|m| m.0 == kind::INPUT).count()
    };
    // A fast PC: a frame every millisecond, aiming all the while.
    let start = Instant::now();
    let mut frames = 0;
    while start.elapsed() < Duration::from_millis(250) {
        let aim = Command {
            yaw: frames as f32 * 0.01,
            ..Command::default()
        };
        client.send_commands(&[(0, aim)]);
        frames += 1;
        std::thread::sleep(Duration::from_millis(1));
    }
    let ticks = start.elapsed().as_secs_f32() / h2sim::game::TICK;
    let sent = inputs(&mut host_end) as f32;
    assert!(
        sent <= ticks + 2.0 && sent >= ticks * 0.5,
        "{sent} for {ticks}"
    );
    assert!(frames as f32 > sent * 2.0);
    // A button pressed goes at once, and so does letting it go.
    let fire = Command {
        fire: true,
        ..Command::default()
    };
    client.send_commands(&[(0, fire)]);
    client.send_commands(&[(0, Command::default())]);
    assert_eq!(inputs(&mut host_end), 2);
}

#[test]
fn online_hosts_send_the_game_30_times_a_second() {
    let world = floor();
    let mut hg = game();
    let shooter = hg.add_player();
    let mut cg = game();
    let who = verified("TESTER", ANY_TEAM);
    let (mut host, mut client) = online("testmap", &cg, "testmap", &[ANY_TEAM], me(), who);
    let mut welcomed = false;
    pump(&mut host, &mut hg, &mut client, &mut cg, |_, ce, cg| {
        welcomed |= matches!(ce.first(), Some(ClientEvent::Welcomed { .. }));
        welcomed && cg.players.len() == 2
    });
    // A tick every frame at 60 frames a second, the host shooting.
    let (before, mut made, mut got) = (client.snapshots, 0, 0);
    let start = Instant::now();
    for k in 0..30 {
        assert!(host.poll(&mut hg, 16).is_empty());
        let mut commands = vec![Command::default(); hg.players.len()];
        commands[shooter].fire = k % 2 == 0;
        hg.step(&world, &commands);
        let events = std::mem::take(&mut hg.events);
        made += events.len();
        host.send(&hg, &events, true);
        got += client.poll(&mut cg).1.len();
        std::thread::sleep(Duration::from_secs_f32(h2sim::game::TICK));
    }
    let seconds = start.elapsed().as_secs_f32();
    let snapshots = (client.snapshots - before) as f32;
    assert!(
        snapshots <= seconds * 30.0 + 2.0,
        "{snapshots} in {seconds} s"
    );
    assert!(snapshots >= seconds * 15.0, "{snapshots} in {seconds} s");
    // What happened between snapshots goes with the next.
    std::thread::sleep(Duration::from_millis(40));
    hg.step(&world, &vec![Command::default(); hg.players.len()]);
    let events = std::mem::take(&mut hg.events);
    made += events.len();
    host.send(&hg, &events, true);
    got += client.poll(&mut cg).1.len();
    assert!(made > 0);
    assert_eq!(got, made);
    assert_eq!(
        cg.players[shooter].body.position,
        hg.players[shooter].body.position
    );
}

// Snapshots as how they differ from the last.

/// The game as a snapshot carries it.
fn state(game: &h2sim::Game) -> Vec<u8> {
    let mut w = h2sim::game::Writer::default();
    game.write_state(&mut w);
    w.0
}

/// The size of a whole snapshot of `game` with `events`.
fn whole(game: &h2sim::Game, events: &[Event]) -> u64 {
    let mut w = h2sim::game::Writer::default();
    for e in events {
        e.write(&mut w);
    }
    (4 + state(game).len() + 2 + w.0.len()) as u64
}

#[test]
fn lan_hosts_send_the_whole_game_every_tick() {
    let world = floor();
    let mut hg = game();
    let shooter = hg.add_player();
    let mut host = Host::bind("testmap", 32, "127.0.0.1:0".parse().unwrap(), false).unwrap();
    let mut cg = game();
    let mut client = Client::connect(address(&host), &cg, "testmap", &[ANY_TEAM], me()).unwrap();
    let mut welcomed = false;
    pump(&mut host, &mut hg, &mut client, &mut cg, |_, ce, cg| {
        welcomed |= matches!(ce.first(), Some(ClientEvent::Welcomed { .. }));
        welcomed && cg.players.len() == 2
    });
    // The joined PC stops reading for a while: on a LAN it still gets
    // every tick's game, whole, as before online play.
    let (sent, snapshots) = (host.sent(), client.snapshots);
    let (mut size, mut made) = (0, 0);
    for k in 0..200 {
        assert!(host.poll(&mut hg, 16).is_empty());
        let mut commands = vec![Command::default(); hg.players.len()];
        commands[shooter].fire = k % 2 == 0;
        hg.step(&world, &commands);
        let events = std::mem::take(&mut hg.events);
        size += whole(&hg, &events);
        made += events.len();
        host.send(&hg, &events, true);
        client.keep_alive();
    }
    assert_eq!(host.sent() - sent, size);
    let mut got = 0;
    let start = Instant::now();
    while client.snapshots < snapshots + 200 {
        let (ce, events) = client.poll(&mut cg);
        assert!(ce.is_empty(), "{ce:?}");
        got += events.len();
        assert!(start.elapsed() < Duration::from_secs(5), "still waiting");
    }
    assert_eq!(client.snapshots, snapshots + 200);
    assert!(made > 0);
    assert_eq!(got, made);
    assert_eq!(state(&cg), state(&hg));
}

/// Bots playing on the test floor: a host with `bots` of them, and a PC
/// joined to it (whose player a bot on the host plays too, unless taken
/// out of `bots`).
struct BotGame {
    world: h2sim::World,
    nav: h2sim::NavGraph,
    hg: h2sim::Game,
    cg: h2sim::Game,
    host: Host,
    client: Client,
    bots: Vec<(usize, h2sim::Bot)>,
}

impl BotGame {
    fn new(bots: usize) -> BotGame {
        BotGame::over(bots, Connection::pair())
    }

    /// The same over another connection: the host's end, then the PC's.
    fn over(bots: usize, (a, b): (Connection, Connection)) -> BotGame {
        let world = floor();
        let points: Vec<glam::Vec3> = (-4..=4)
            .flat_map(|x| {
                (-4..=4).map(move |y| glam::Vec3::new(x as f32 * 3.0, y as f32 * 3.0, 0.0))
            })
            .collect();
        let nav = h2sim::NavGraph::build(&world, &points);
        let mut hg = game();
        hg.rules.score_to_win = 0;
        for _ in 0..bots {
            hg.add_player();
        }
        let mut cg = game();
        let mut host = Host::online("testmap");
        host.add_connection(a, verified("TESTER", ANY_TEAM));
        let mut client = Client::over(b, &cg, "testmap", &[ANY_TEAM], me());
        let mut welcomed = false;
        pump(&mut host, &mut hg, &mut client, &mut cg, |_, ce, _| {
            welcomed |= matches!(ce.first(), Some(ClientEvent::Welcomed { .. }));
            welcomed
        });
        let bots = (0..hg.players.len())
            .map(|i| (i, h2sim::Bot::new(i as u32 * 31 + 5)))
            .collect();
        BotGame {
            world,
            nav,
            hg,
            cg,
            host,
            client,
            bots,
        }
    }

    /// Run a tick on the host and send it. What happened in it.
    fn tick(&mut self) -> Vec<Event> {
        let mut commands: Vec<Command> = (0..self.hg.players.len())
            .map(|i| self.host.command(i).unwrap_or_default())
            .collect();
        for (i, bot) in &mut self.bots {
            commands[*i] = bot.think(&self.hg, &self.world, &self.nav, *i);
        }
        self.hg.step(&self.world, &commands);
        let events = std::mem::take(&mut self.hg.events);
        self.host.send(&self.hg, &events, true);
        events
    }
}

#[test]
fn delta_snapshots_rebuild_the_hosts_game() {
    let mut g = BotGame::new(5);
    // Every tick, to check every snapshot.
    g.host.set_rate(0);
    let (mut full, mut events_made, mut events_got) = (0, 0, 0);
    let sent = g.host.sent();
    for _ in 0..1000 {
        assert!(g.host.poll(&mut g.hg, 16).is_empty());
        let events = g.tick();
        // A whole snapshot: its number, the game, and what happened.
        let mut w = h2sim::game::Writer::default();
        for e in &events {
            e.write(&mut w);
        }
        full += 4 + state(&g.hg).len() + 2 + w.0.len();
        events_made += events.len();
        let (ce, got) = g.client.poll(&mut g.cg);
        assert!(ce.is_empty(), "{ce:?}");
        assert_eq!(got, events);
        events_got += got.len();
        // Rebuilt from the change, the game is as the host has it.
        assert_eq!(state(&g.cg), state(&g.hg));
    }
    assert!(events_made > 100, "{events_made}");
    assert_eq!(events_got, events_made);
    let sent = (g.host.sent() - sent) as usize;
    println!(
        "{} players: {} bytes a snapshot in full, {} as changes",
        g.hg.players.len(),
        full / 1000,
        sent / 1000
    );
    assert!(sent * 10 < full * 4, "{sent} bytes for {full}");
}

#[test]
fn a_pc_behind_misses_snapshots_but_not_what_happened() {
    let mut g = BotGame::new(5);
    g.host.set_rate(0);
    // The joined PC stops reading for a while: the host stops sending it
    // the game once it's well behind.
    let mut made = Vec::new();
    let mut sent = Vec::new();
    for _ in 0..1000 {
        let before = g.host.sent();
        made.extend(g.tick());
        sent.push(g.host.sent() - before);
        assert!(g.host.poll(&mut g.hg, 16).is_empty());
    }
    let skipped = sent.iter().rev().take_while(|&&n| n == 0).count();
    assert!(skipped > 100, "{skipped}");
    // It catches up: what happened meanwhile comes with the whole game.
    let mut got = Vec::new();
    for _ in 0..50 {
        let (ce, events) = g.client.poll(&mut g.cg);
        assert!(ce.is_empty(), "{ce:?}");
        got.extend(events);
        assert!(g.host.poll(&mut g.hg, 16).is_empty());
        made.extend(g.tick());
    }
    let (_, events) = g.client.poll(&mut g.cg);
    got.extend(events);
    assert_eq!(got, made);
    assert_eq!(state(&g.cg), state(&g.hg));
    assert!(g.client.snapshots < 1050);
}

/// The two ends of a TCP connection over loopback: the end a listener
/// took, and the end that connected to it.
fn tcp_pair() -> (Connection, Connection) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let joining = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (taken, _) = listener.accept().unwrap();
    let tcp = |s| Connection::tcp(s).unwrap();
    (tcp(taken), tcp(joining))
}

#[test]
fn a_pc_behind_deep_buffers_misses_snapshots() {
    behind_deep_buffers(tcp_pair());
}

#[test]
fn a_websocket_pc_behind_deep_buffers_misses_snapshots() {
    behind_deep_buffers(ws_pair());
}

fn behind_deep_buffers((taken, joining): (Connection, Connection)) {
    // Online, the game can pile up on its way (in the network's buffers, a
    // relay's) while the host's own outbox stays empty.
    let world = floor();
    let mut hg = game();
    let shooter = hg.add_player();
    let mut host = Host::online("testmap");
    host.set_rate(0);
    host.add_connection(taken, verified("TESTER", ANY_TEAM));
    let mut cg = game();
    let mut client = Client::over(joining, &cg, "testmap", &[ANY_TEAM], me());
    let mut welcomed = false;
    pump(&mut host, &mut hg, &mut client, &mut cg, |_, ce, cg| {
        welcomed |= matches!(ce.first(), Some(ClientEvent::Welcomed { .. }));
        welcomed && cg.players.len() == 2
    });
    let tick = |k: usize, host: &mut Host, hg: &mut h2sim::Game| {
        assert!(host.poll(hg, 16).is_empty());
        let mut commands = vec![Command::default(); hg.players.len()];
        commands[shooter].fire = k.is_multiple_of(2);
        hg.step(&world, &commands);
        let events = std::mem::take(&mut hg.events);
        host.send(hg, &events, true);
        events
    };
    // The joined PC stops reading: soon nothing more goes to it.
    let mut made = Vec::new();
    let mut went = 0;
    for k in 0..300 {
        let before = host.sent();
        made.extend(tick(k, &mut host, &mut hg));
        went += (host.sent() > before) as usize;
        client.keep_alive();
    }
    assert!(went < 50, "{went} of 300 snapshots went");
    // It reads again and catches up, missing nothing that happened.
    let mut got = Vec::new();
    for k in 0..30 {
        let (ce, events) = client.poll(&mut cg);
        assert!(ce.is_empty(), "{ce:?}");
        got.extend(events);
        made.extend(tick(k, &mut host, &mut hg));
        std::thread::sleep(Duration::from_millis(2));
    }
    got.extend(client.poll(&mut cg).1);
    assert_eq!(got, made);
    assert_eq!(state(&cg), state(&hg));
}

#[test]
fn a_pc_that_never_catches_up_is_dropped() {
    let mut g = BotGame::new(3);
    g.host.set_rate(0);
    g.host.set_timeout(SHORT);
    g.client.set_timeout(SHORT);
    // Its window is minimized, say: it says it's there, but reads nothing.
    let start = Instant::now();
    let reason = loop {
        g.tick();
        g.client.keep_alive();
        if let Some(HostEvent::Left { reason, .. }) = g.host.poll(&mut g.hg, 16).first() {
            break reason.clone();
        }
        assert!(start.elapsed() < SHORT * 10, "still there");
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(reason, "connection too slow");
}

#[test]
fn messages_put_back_come_again_before_the_end_goes() {
    let (mut a, mut b) = Connection::pair();
    for kind in [1, 2, 3] {
        a.send(kind, &[kind]);
    }
    a.flush().unwrap();
    drop(a);
    let mut got = b.receive().unwrap();
    assert_eq!(got.len(), 3);
    b.put_back(got.split_off(1));
    assert_eq!(b.receive().unwrap(), vec![(2, vec![2]), (3, vec![3])]);
    assert!(b.receive().is_err());
}

// Under lag, as over the internet: every message 100-200 ms late.

/// Timeouts for games under lag: ten heartbeats, against messages up to a
/// fifth of a second late.
const LAGGED: Duration = Duration::from_secs(1);

/// Messages on their way, each with when it arrives.
type Late = VecDeque<(Instant, u8, Vec<u8>)>;

/// The two ends of a connection (a host's, and a joining PC's) on which
/// each message takes 100-200 ms to arrive, in the order sent.
fn lag_pair() -> (Connection, Connection) {
    let (host_end, mut a) = Connection::pair();
    let (mut b, pc_end) = Connection::pair();
    std::thread::spawn(move || {
        let (mut to_b, mut to_a, mut seed) = (Late::new(), Late::new(), 1);
        // Until either end goes.
        while carry(&mut a, &mut b, &mut to_b, &mut seed)
            .and_then(|()| carry(&mut b, &mut a, &mut to_a, &mut seed))
            .is_ok()
        {
            std::thread::sleep(Duration::from_millis(1));
        }
    });
    (host_end, pc_end)
}

/// Take what `from` sent, a random 100-200 ms late, and hand `to` what's
/// due.
fn carry(
    from: &mut Connection,
    to: &mut Connection,
    late: &mut Late,
    seed: &mut u32,
) -> Result<(), String> {
    let now = Instant::now();
    for (kind, body) in from.receive()? {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let lag = Duration::from_millis(100 + (*seed >> 16) as u64 % 101);
        // Never ahead of what was sent before it.
        let due = late.back().map_or(now, |m| m.0).max(now + lag);
        late.push_back((due, kind, body));
    }
    while late.front().is_some_and(|m| m.0 <= now) {
        let (_, kind, body) = late.pop_front().unwrap();
        to.send(kind, &body);
    }
    to.flush()
}

#[test]
fn an_idle_lobby_stays_connected_under_lag() {
    let mut hg = game();
    hg.add_player();
    let mut host = Host::online("lockout");
    host.set_timeout(LAGGED);
    host.set_lobby(Lobby::default());
    let (a, b) = lag_pair();
    host.add_connection(a, verified("TESTER", ANY_TEAM));
    let mut cg = game();
    let mut client = Client::over(b, &cg, "midship", &[ANY_TEAM], me());
    client.set_timeout(LAGGED);
    // Three timeouts with nothing to say but that they're still there.
    let start = Instant::now();
    let mut seen = false;
    while start.elapsed() < LAGGED * 3 {
        for e in host.poll(&mut hg, 16) {
            assert!(matches!(e, HostEvent::Arrived { .. }), "{e:?}");
        }
        for e in client.poll(&mut cg).0 {
            assert!(matches!(e, ClientEvent::Lobby(_)), "{e:?}");
            seen = true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(seen);
    assert_eq!(host.joined(), 1);
}

#[test]
fn a_game_under_lag_goes_on() {
    let mut g = BotGame::over(5, lag_pair());
    g.host.set_timeout(LAGGED);
    g.client.set_timeout(LAGGED);
    // A snapshot every tick, a tick every 30th of a second, as online.
    g.host.set_rate(0);
    // The joined PC plays its own player, firing all the while.
    let mine = g.client.players[0];
    g.bots.retain(|b| b.0 != mine);
    let fire = Command {
        fire: true,
        ..Command::default()
    };
    let (mut made, mut got, mut shots) = (Vec::new(), Vec::new(), 0);
    for _ in 0..60 {
        g.client.send_commands(&[(mine, fire)]);
        assert!(g.host.poll(&mut g.hg, 16).is_empty());
        let before = g.host.sent();
        let events = g.tick();
        // Each goes: lag alone doesn't put the PC behind.
        assert!(g.host.sent() > before);
        shots += events
            .iter()
            .filter(|e| matches!(e, Event::Shot { player, .. } if *player == mine))
            .count();
        made.extend(events);
        let (ce, events) = g.client.poll(&mut g.cg);
        assert!(ce.is_empty(), "{ce:?}");
        got.extend(events);
        std::thread::sleep(Duration::from_secs_f32(2.0 * h2sim::game::TICK));
    }
    // Its controls reached the host.
    assert!(shots > 0);
    // And the last of the game reaches it.
    let start = Instant::now();
    while got.len() < made.len() || state(&g.cg) != state(&g.hg) {
        assert!(g.host.poll(&mut g.hg, 16).is_empty());
        let (ce, events) = g.client.poll(&mut g.cg);
        assert!(ce.is_empty(), "{ce:?}");
        got.extend(events);
        assert!(start.elapsed() < LAGGED, "still waiting");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(got, made);
}

// WebSockets: dialing a server, what a server takes, and the limits.

#[test]
fn dialing_where_no_server_listens_fails_at_once() {
    // A port nothing listens on (any more).
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/live", listener.local_addr().unwrap());
    drop(listener);
    let start = Instant::now();
    let dialing = dial(&url, Duration::from_secs(10));
    // The game goes on meanwhile.
    assert!(start.elapsed() < Duration::from_millis(100));
    let Err(why) = dialing.recv_timeout(Duration::from_secs(2)).unwrap() else {
        panic!("connected");
    };
    assert!(why.starts_with("couldn't connect to 127.0.0.1: "), "{why}");
}

#[test]
fn dialing_a_bad_address_fails() {
    for (url, expected) in [
        ("lockout", "not a server address: lockout"),
        (
            "http://example.com/live",
            "not a server address: http://example.com/live",
        ),
        (
            "ws://no-such-host.invalid/live",
            "couldn't find no-such-host.invalid",
        ),
        // Not port 80, for want of a port that's one.
        (
            "ws://127.0.0.1:65616/live",
            "not a server address: ws://127.0.0.1:65616/live",
        ),
    ] {
        let Err(why) = dial(url, Duration::from_secs(5)).recv().unwrap() else {
            panic!("{url} connected");
        };
        assert_eq!(why, expected);
    }
}

#[test]
fn dialing_a_server_that_never_answers_gives_up_in_time() {
    // The system takes the connection for it, but it says nothing.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/live", listener.local_addr().unwrap());
    let start = Instant::now();
    let Err(why) = dial(&url, SHORT).recv().unwrap() else {
        panic!("connected");
    };
    assert_eq!(why, "127.0.0.1 took too long to answer");
    let took = start.elapsed();
    assert!(took >= SHORT && took < SHORT * 3, "{took:?}");
}

#[test]
fn dialing_a_server_that_answers_ever_so_slowly_gives_up_in_time() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/live", listener.local_addr().unwrap());
    let start = Instant::now();
    let dialing = dial(&url, SHORT);
    let (mut stream, _) = listener.accept().unwrap();
    std::thread::spawn(move || {
        Read::read(&mut stream, &mut [0; 4096]).unwrap();
        // A byte at a time, each soon after the last.
        for b in b"HTTP/1.1 101 Switching Protocols\r\n" {
            if stream.write_all(&[*b]).is_err() {
                break;
            }
            std::thread::sleep(SHORT / 6);
        }
    });
    let Err(why) = dialing.recv().unwrap() else {
        panic!("connected");
    };
    assert_eq!(why, "127.0.0.1 took too long to answer");
    let took = start.elapsed();
    assert!(took < SHORT * 2, "{took:?}");
}

#[test]
fn dialing_leaves_time_for_a_names_other_addresses() {
    use socket2::{Domain, Socket, Type};
    // One that leads nowhere: a listener with no room for more connections
    // drops what asks for one.
    let nowhere = Socket::new(Domain::IPV4, Type::STREAM, None).unwrap();
    nowhere
        .bind(&SocketAddr::from((Ipv4Addr::LOCALHOST, 0)).into())
        .unwrap();
    nowhere.listen(0).unwrap();
    let nowhere = nowhere.local_addr().unwrap().as_socket().unwrap();
    let _filling = std::net::TcpStream::connect(nowhere).unwrap();
    // Then one that works.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addresses = [nowhere, listener.local_addr().unwrap()];
    let start = Instant::now();
    let reached = ws::reach("game.example", &addresses, start + SHORT);
    assert!(reached.is_ok(), "{:?}", reached.err());
    assert!(start.elapsed() < SHORT, "{:?}", start.elapsed());
}

#[test]
fn dialing_takes_schemes_in_any_case() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    for url in [
        format!("WS://{address}/live"),
        format!("Ws://{address}/live"),
    ] {
        let dialing = dial(&url, Duration::from_secs(5));
        let (stream, _) = listener.accept().unwrap();
        assert!(
            matches!(accept(stream), Ok(Request::WebSocket(..))),
            "{url}"
        );
        assert!(dialing.recv().unwrap().is_ok(), "{url}");
    }
}

#[test]
fn dialing_a_web_server_that_isnt_a_game_server_fails() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/live", listener.local_addr().unwrap());
    let dialing = dial(&url, Duration::from_secs(5));
    let (mut stream, _) = listener.accept().unwrap();
    std::io::Read::read(&mut stream, &mut [0; 4096]).unwrap();
    reply(stream, "404 Not Found", "").unwrap();
    let Err(why) = dialing.recv().unwrap() else {
        panic!("connected");
    };
    assert_eq!(why, "127.0.0.1 isn't a game server (404 Not Found)");
}

/// Wait while `from` sends what it has until `to` has had `n` messages, or
/// its connection fails (why, then).
fn deliver(
    from: &mut Connection,
    to: &mut Connection,
    n: usize,
) -> Result<Vec<conn::Message>, String> {
    let mut got = Vec::new();
    let start = Instant::now();
    while got.len() < n {
        let _ = from.flush();
        got.extend(to.receive()?);
        assert!(start.elapsed() < Duration::from_secs(5), "still waiting");
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(got)
}

#[test]
fn servers_answer_web_requests_and_take_websockets() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    // A browser, or a health check.
    let browser = std::thread::spawn(move || {
        let mut s = std::net::TcpStream::connect(address).unwrap();
        s.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut answer = String::new();
        s.read_to_string(&mut answer).unwrap();
        answer
    });
    let (stream, _) = listener.accept().unwrap();
    let Ok(Request::Http(stream, path)) = accept(stream) else {
        panic!("no web request");
    };
    assert_eq!(path, "/health");
    reply(stream, "200 OK", "3 PLAYERS ONLINE").unwrap();
    let answer = browser.join().unwrap();
    assert!(answer.starts_with("HTTP/1.1 200 OK\r\n"), "{answer}");
    assert!(answer.ends_with("\r\n\r\n3 PLAYERS ONLINE"), "{answer}");

    // A game, on the same port.
    let dialing = dial(&format!("ws://{address}/live"), Duration::from_secs(5));
    let (stream, _) = listener.accept().unwrap();
    let Ok(Request::WebSocket(mut taken, path)) = accept(stream) else {
        panic!("no WebSocket");
    };
    assert_eq!(path, "/live");
    let mut dialed = dialing.recv().unwrap().unwrap();
    dialed.send(7, b"hello");
    let got = deliver(&mut dialed, &mut taken, 1);
    assert_eq!(got, Ok(vec![(7, b"hello".to_vec())]));

    // Something else altogether.
    let mut s = std::net::TcpStream::connect(address).unwrap();
    s.write_all(b"H2RS\x16\0\0\0lockout\r\n\r\n").unwrap();
    let (stream, _) = listener.accept().unwrap();
    assert_eq!(accept(stream).err().as_deref(), Some("not a web request"));
}

#[test]
fn servers_see_just_the_path() {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    // Without the query...
    let dialing = dial(&format!("ws://{address}/live?v=1"), Duration::from_secs(5));
    let (stream, _) = listener.accept().unwrap();
    let Ok(Request::WebSocket(_, path)) = accept(stream) else {
        panic!("no WebSocket");
    };
    assert_eq!(path, "/live");
    assert!(dialing.recv().unwrap().is_ok());
    // ...or the scheme and host, as a proxy may send them.
    let mut s = std::net::TcpStream::connect(address).unwrap();
    s.write_all(b"GET http://localhost/health?all HTTP/1.1\r\n\r\n")
        .unwrap();
    let (stream, _) = listener.accept().unwrap();
    let Ok(Request::Http(_, path)) = accept(stream) else {
        panic!("no web request");
    };
    assert_eq!(path, "/health");
}

#[test]
fn servers_give_up_on_requests_that_never_end_in_time() {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let mut s = std::net::TcpStream::connect(address).unwrap();
        // A byte at a time, each soon after the last.
        for b in b"GET /live HTTP/1.1\r\nHost: localhost\r\n" {
            if s.write_all(&[*b]).is_err() {
                break;
            }
            std::thread::sleep(SHORT / 6);
        }
    });
    let (stream, _) = listener.accept().unwrap();
    let start = Instant::now();
    let why = ws::accept_within(stream, SHORT).err();
    assert_eq!(why.as_deref(), Some("never said what it wants"));
    let took = start.elapsed();
    assert!(took < SHORT * 2, "{took:?}");
}

#[test]
fn servers_wait_for_the_request_on_connections_that_never_wait() {
    use std::io::Write;
    // As on Windows, where what a listener that never waits takes doesn't
    // either.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let browser = std::thread::spawn(move || {
        let mut s = std::net::TcpStream::connect(address).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        s.write_all(b"GET /health HTTP/1.1\r\n\r\n").unwrap();
    });
    let (stream, _) = listener.accept().unwrap();
    stream.set_nonblocking(true).unwrap();
    let Ok(Request::Http(_, path)) = accept(stream) else {
        panic!("no web request");
    };
    assert_eq!(path, "/health");
    browser.join().unwrap();
}

#[test]
fn a_websocket_asked_for_wrongly_is_answered() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut s = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    s.write_all(b"GET /live HTTP/1.1\r\nUpgrade: websocket\r\nConnection: keep-alive\r\n\r\n")
        .unwrap();
    let (stream, _) = listener.accept().unwrap();
    let why = accept(stream).err();
    let expected = "WebSocket protocol error: No \"Connection: upgrade\" header";
    assert_eq!(why.as_deref(), Some(expected));
    let mut answer = String::new();
    s.read_to_string(&mut answer).unwrap();
    assert!(
        answer.starts_with("HTTP/1.1 400 Bad Request\r\n"),
        "{answer}"
    );
}

#[test]
fn the_largest_messages_arrive_over_tcp() {
    the_largest_messages_arrive(tcp_pair());
}

#[test]
fn the_largest_messages_arrive_by_websocket() {
    the_largest_messages_arrive(ws_pair());
}

/// Messages up to a mebibyte (kind and body) arrive whole, both ways; a
/// larger one means a broken or hostile PC, and the end of the connection.
fn the_largest_messages_arrive((mut a, mut b): (Connection, Connection)) {
    let largest: Vec<u8> = (0..conn::MAX_MESSAGE - 1).map(|i| i as u8).collect();
    a.send(9, &largest);
    a.send(1, &[]);
    let got = deliver(&mut a, &mut b, 2);
    assert!(got == Ok(vec![(9, largest.clone()), (1, Vec::new())]));
    b.send(9, &largest);
    assert!(deliver(&mut b, &mut a, 1) == Ok(vec![(9, largest)]));
    a.send(9, &vec![0; conn::MAX_MESSAGE]);
    assert_eq!(deliver(&mut a, &mut b, 1), Err("bad message".into()));
}

#[test]
fn a_websocket_that_takes_nothing_is_too_slow() {
    let (mut taken, _dialed) = ws_pair();
    // The PC never reads: the network holds what it can, then what's
    // waiting to go grows past the limit.
    let chunk = vec![0; 64 << 10];
    let mut sent = 0;
    let why = loop {
        taken.send(1, &chunk);
        sent += chunk.len() + 1;
        if let Err(why) = taken.flush() {
            break why;
        }
        assert!(sent < 256 << 20, "{sent} bytes sent");
    };
    assert_eq!(why, "connection too slow");
    assert!(sent > conn::MAX_BACKLOG, "{sent} bytes sent");
}

#[test]
fn a_websocket_that_pings_and_takes_nothing_is_too_slow() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    // Something that asks for an answer over and over, and never reads one.
    let pinging = std::thread::spawn(move || {
        let tcp = std::net::TcpStream::connect(address).unwrap();
        let (mut ws, _) = tungstenite::client(format!("ws://{address}/live"), tcp).unwrap();
        for _ in 0..500_000 {
            let ping = tungstenite::Message::Ping(vec![0; 125].into());
            if ws.write(ping).is_err() {
                break;
            }
        }
    });
    let (stream, _) = listener.accept().unwrap();
    let Ok(Request::WebSocket(mut taken, _)) = accept(stream) else {
        panic!("no WebSocket");
    };
    // The answers wait to go out, as more of what it's behind by.
    let why = loop {
        if let Err(why) = taken.receive().and_then(|_| taken.flush()) {
            break why;
        }
    };
    assert_eq!(why, "connection too slow");
    drop(taken);
    pinging.join().unwrap();
}

/// Why `conn` ends, once it does.
fn end_of(conn: &mut Connection) -> String {
    let start = Instant::now();
    loop {
        if let Err(why) = conn.receive() {
            return why;
        }
        assert!(start.elapsed() < Duration::from_secs(5), "still there");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn a_websocket_reset_is_just_closed() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/live", listener.local_addr().unwrap());
    let dialing = dial(&url, Duration::from_secs(5));
    let (stream, _) = listener.accept().unwrap();
    tungstenite::accept(&stream).unwrap();
    let mut dialed = dialing.recv().unwrap().unwrap();
    let linger = socket2::SockRef::from(&stream).set_linger(Some(Duration::ZERO));
    linger.unwrap();
    drop(stream);
    assert_eq!(end_of(&mut dialed), "connection closed");
}

// Secure WebSockets, to a server behind TLS as online servers are (with a
// certificate of the test's own).

type SecureWebSocket =
    tungstenite::WebSocket<rustls::StreamOwned<rustls::ServerConnection, std::net::TcpStream>>;

/// A secure server on loopback, as `name`, that sends back every message
/// it gets: where it is, and the certificate to trust to reach it.
fn secure_echo(name: &str) -> (SocketAddr, rustls::RootCertStore) {
    secure_server(name, |ws| {
        while let Ok(m) = ws.read() {
            if m.is_binary() && ws.send(m).is_err() {
                break;
            }
        }
    })
}

/// A secure server on loopback, as `name`, that has `serve` take each
/// WebSocket: where it is, and the certificate to trust to reach it.
fn secure_server(
    name: &str,
    serve: fn(&mut SecureWebSocket),
) -> (SocketAddr, rustls::RootCertStore) {
    use std::sync::Arc;
    let certified = rcgen::generate_simple_self_signed([name.to_string()]).unwrap();
    let cert = certified.cert.der().clone();
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert.clone()], key.into())
        .unwrap();
    let config = Arc::new(config);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for tcp in listener.incoming().flatten() {
            let tls = rustls::ServerConnection::new(config.clone()).unwrap();
            if let Ok(mut ws) = tungstenite::accept(rustls::StreamOwned::new(tls, tcp)) {
                serve(&mut ws);
            }
        }
    });
    let mut roots = rustls::RootCertStore::empty();
    roots.add(cert).unwrap();
    (address, roots)
}

#[test]
fn secure_websockets_carry_messages() {
    let (address, roots) = secure_echo("localhost");
    let url = format!("wss://localhost:{}/live", address.port());
    let dialing = ws::dial_trusting(&url, Duration::from_secs(5), roots);
    let mut conn = dialing.recv().unwrap().unwrap();
    // Messages of every size up to the largest, more at a time than the
    // network holds (so it backs up both ways), and they all come back.
    let largest = conn::MAX_MESSAGE - 1;
    let sizes = [
        0, 1, 1000, 100_000, largest, largest, largest, largest, largest,
    ];
    for round in 0..3u8 {
        let sent: Vec<conn::Message> = sizes
            .iter()
            .enumerate()
            .map(|(k, &n)| (round * 10 + k as u8, vec![k as u8 ^ round; n]))
            .collect();
        for (kind, body) in &sent {
            conn.send(*kind, body);
        }
        let mut got = Vec::new();
        let start = Instant::now();
        while got.len() < sent.len() {
            conn.flush().unwrap();
            got.extend(conn.receive().unwrap());
            assert!(start.elapsed() < Duration::from_secs(5), "still waiting");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(got == sent, "round {round}");
    }
}

#[test]
fn an_untrusted_secure_server_is_refused() {
    let (address, _) = secure_echo("localhost");
    let url = format!("wss://localhost:{}/live", address.port());
    let Err(why) = dial(&url, Duration::from_secs(5)).recv().unwrap() else {
        panic!("connected");
    };
    assert!(
        why.starts_with("couldn't connect to localhost: invalid peer certificate"),
        "{why}"
    );
}

#[test]
fn a_secure_server_gone_without_a_word_is_just_gone() {
    // Without TLS's goodbye, as when a server online crashes, or what it
    // runs behind restarts.
    let (address, roots) = secure_server("localhost", |ws| {
        let _ = ws.send(tungstenite::Message::Binary(vec![7].into()));
    });
    let url = format!("wss://localhost:{}/live", address.port());
    let dialing = ws::dial_trusting(&url, Duration::from_secs(5), roots);
    let mut dialed = dialing.recv().unwrap().unwrap();
    assert_eq!(end_of(&mut dialed), "connection closed");
}
