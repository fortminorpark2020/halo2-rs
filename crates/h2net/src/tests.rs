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
