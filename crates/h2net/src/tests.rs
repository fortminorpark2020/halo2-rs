use super::*;
use h2sim::game::{Emblem, Event, Look};
use h2sim::testing::{floor, game};
use h2sim::Command;
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

// The same over connections in memory, as online games join through the
// service (no ports at all), and a host on any free port.

/// An online host and a PC joining it, verified as `who`.
fn online(
    map: &str,
    cg: &h2sim::Game,
    client_map: &str,
    teams: &[u8],
    me: (&str, Look),
    who: Verified,
) -> (Host, Client) {
    let mut host = Host::online(map);
    let (a, b) = Connection::pair();
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
    let (mut host, mut client) = online(
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
    // The last tick goes with the next snapshot.
    std::thread::sleep(Duration::from_millis(40));
    host.send(&hg, &[], false);
    client.poll(&mut cg);
    assert!(hg.players[mine].body.position.x > start.x + 0.3);
    assert!(shots > 0);
    let joined = cg.players[mine].body.position;
    assert_eq!(joined, hg.players[mine].body.position);

    // Leaving hands the Spartan back to the host.
    drop(client);
    let events = host.poll(&mut hg, 16);
    let Some(HostEvent::Left { players, .. }) = events.first() else {
        panic!("{events:?}");
    };
    assert_eq!(players, &vec![mine]);
    assert!(!host.is_remote(mine));
}

#[test]
fn online_taps_between_ticks_are_not_lost() {
    let mut hg = game();
    hg.add_player();
    let mut cg = game();
    let who = verified("TESTER", ANY_TEAM);
    let (mut host, mut client) = online("testmap", &cg, "testmap", &[ANY_TEAM], me(), who);
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
    host.poll(&mut hg, 16);
    // Pressed for one tick, then released.
    assert_eq!(host.command(mine), Some(tap));
    assert_eq!(host.command(mine), Some(Command::default()));
}

#[test]
fn an_online_pc_on_another_map_is_turned_away() {
    let mut hg = game();
    hg.add_player();
    let mut cg = game();
    let who = verified("TESTER", ANY_TEAM);
    let (mut host, mut client) = online("lockout", &cg, "midship", &[ANY_TEAM], me(), who);
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
    let (a, b) = Connection::pair();
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

// Heartbeats, with a short timeout instead of the real ten seconds.

const SHORT: Duration = Duration::from_millis(300);

#[test]
fn a_silent_host_is_lost_within_the_timeout() {
    let cg = game();
    let (mut silent, end) = Connection::pair();
    let mut client = Client::over(end, &cg, "testmap", &[ANY_TEAM], me());
    client.set_timeout(SHORT);
    let mut cg = game();
    let start = Instant::now();
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

/// Bots playing on the test floor: a host with `bots` of them, and a PC
/// joined to it (whose player a bot on the host plays too).
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
        let who = verified("TESTER", ANY_TEAM);
        let (mut host, mut client) = online("testmap", &cg, "testmap", &[ANY_TEAM], me(), who);
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
        let mut commands = vec![Command::default(); self.hg.players.len()];
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
        made.extend(g.tick());
    }
    let (_, events) = g.client.poll(&mut g.cg);
    got.extend(events);
    assert_eq!(got, made);
    assert_eq!(state(&g.cg), state(&g.hg));
    assert!(g.client.snapshots < 1050);
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
