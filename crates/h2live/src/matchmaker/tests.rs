use super::*;
use crate::playlists::{built_in, parse};
use h2sim::GameType;
use std::collections::HashSet;

// The built-in playlists by id.
const RUMBLE_PIT: u8 = 0;
const HEAD_TO_HEAD: u8 = 1;
const DOUBLE_TEAM: u8 = 2;
const TEAM_SLAYER: u8 = 3;
const TEAM_SKIRMISH: u8 = 4;
const BIG_TEAM: u8 = 5;
const TEAM_TRAINING: u8 = 6;
const TEAM_SNIPERS: u8 = 7;
const TEAM_HARDCORE: u8 = 8;
// And the launcher's.
const MCC_RUMBLE_PIT: u8 = 10;
const MCC_HEAD_TO_HEAD: u8 = 11;
const MCC_TEAM_SLAYER: u8 = 13;

/// Every map in the built-in playlists, everyone's copy the same.
fn every_map() -> Vec<(String, u64)> {
    let mut maps: Vec<(String, u64)> = built_in()
        .into_iter()
        .flat_map(|p| p.maps)
        .map(|m| (m, 7))
        .collect();
    maps.sort();
    maps.dedup();
    maps
}

/// A player at `level` in every playlist, with every map and a 50 ms round
/// trip.
fn member(account: u64, level: u8) -> Member {
    Member {
        account,
        levels: built_in().iter().map(|p| (p.id, level)).collect(),
        guests: 0,
        maps: every_map(),
        rtt: 50,
    }
}

/// A player with a round trip of `rtt` milliseconds.
fn pc(account: u64, rtt: u32) -> Member {
    Member {
        rtt,
        ..member(account, 5)
    }
}

/// `n` players at level 5, numbered from `from`.
fn players(from: u64, n: u64) -> Vec<Member> {
    (from..from + n).map(|a| member(a, 5)).collect()
}

fn party(party: u64, playlist: u8, members: Vec<Member>) -> Ticket {
    Ticket {
        party,
        client: ClientKind::Viewer,
        playlist,
        members,
        previous_map: None,
    }
}

/// A party of launchers.
fn launchers(id: u64, playlist: u8, members: Vec<Member>) -> Ticket {
    Ticket {
        client: ClientKind::Launcher,
        ..party(id, playlist, members)
    }
}

/// A party of one, whose account has the party's number.
fn solo(id: u64, playlist: u8, level: u8) -> Ticket {
    party(id, playlist, vec![member(id, level)])
}

fn matchmaker() -> Matchmaker {
    Matchmaker::new(built_in(), 1)
}

/// Poll every quarter second from `from` to `to`, both included, and return
/// what happened when.
fn run(mm: &mut Matchmaker, from: f64, to: f64) -> Vec<(f64, Event)> {
    let mut events = Vec::new();
    let mut k = (from / TICK).round() as u32;
    while k as f64 * TICK <= to {
        let t = k as f64 * TICK;
        events.extend(mm.poll(t).into_iter().map(|e| (t, e)));
        k += 1;
    }
    events
}

/// The matches formed, and when.
fn formed(events: &[(f64, Event)]) -> Vec<(f64, Match)> {
    let matches = events.iter().filter_map(|(t, e)| match e {
        Event::Formed(m) => Some((*t, m.clone())),
        _ => None,
    });
    matches.collect()
}

/// The statuses `party` was shown, and when.
fn statuses(events: &[(f64, Event)], party: u64) -> Vec<(f64, Status)> {
    let shown = events.iter().filter_map(|(t, e)| match e {
        Event::Status { party: p, status } if *p == party => Some((*t, *status)),
        _ => None,
    });
    shown.collect()
}

/// What `party` was showing at time `t`.
fn status_at(events: &[(f64, Event)], party: u64, t: f64) -> Status {
    let shown = statuses(events, party);
    shown.iter().rev().find(|(when, _)| *when <= t).unwrap().1
}

fn accounts(m: &Match) -> Vec<u64> {
    m.players.iter().map(|p| p.account).collect()
}

fn team(m: &Match, account: u64) -> u8 {
    m.players
        .iter()
        .find(|p| p.account == account)
        .unwrap()
        .team
}

#[test]
fn two_players_make_a_head_to_head_match_at_once() {
    let mut mm = matchmaker();
    assert_eq!(mm.search(solo(1, HEAD_TO_HEAD, 5), 0.0), Ok(HEAD_TO_HEAD));
    assert_eq!(mm.search(solo(2, HEAD_TO_HEAD, 7), 0.0), Ok(HEAD_TO_HEAD));
    assert_eq!(mm.searching(HEAD_TO_HEAD), 2);
    let matches = formed(&run(&mut mm, 0.0, 0.0));
    assert_eq!(matches.len(), 1);
    let m = &matches[0].1;
    assert_eq!((m.playlist, m.ranked, m.bots), (HEAD_TO_HEAD, true, 0));
    assert_eq!(accounts(m), [1, 2]);
    assert!(m.players.iter().all(|p| p.team == 0));
    assert_eq!((m.players[1].level, m.players[1].effective), (7, 7));
    let playlist = &built_in()[1];
    assert!(playlist.maps.contains(&m.map));
    assert!(playlist.variants.contains(&m.variant));
    assert_eq!(m.hash, 7);
    // They're not searching any more.
    assert_eq!(mm.searching(HEAD_TO_HEAD), 0);
    assert!(run(&mut mm, 0.25, 30.0).is_empty());
}

#[test]
fn searches_run_four_times_a_second() {
    let mut mm = matchmaker();
    assert!(mm.poll(0.0).is_empty());
    mm.search(solo(1, HEAD_TO_HEAD, 5), 0.1).unwrap();
    mm.search(solo(2, HEAD_TO_HEAD, 5), 0.1).unwrap();
    assert!(mm.poll(0.1).is_empty());
    assert_eq!(formed(&run(&mut mm, 0.25, 0.25)).len(), 1);
}

#[test]
fn a_match_with_its_fewest_players_counts_down_twenty_seconds() {
    let mut mm = matchmaker();
    for id in 1..=3 {
        mm.search(solo(id, RUMBLE_PIT, 5), 0.0).unwrap();
    }
    let events = run(&mut mm, 0.0, 19.75);
    assert!(formed(&events).is_empty());
    let waiting = Status {
        stage: Stage::WaitingToFill,
        have: 3,
        need: 0,
        seconds: 20,
        low: 1,
        high: 11,
    };
    assert_eq!(status_at(&events, 1, 0.0), waiting);
    assert_eq!(status_at(&events, 3, 10.0).seconds, 10);
    assert_eq!(status_at(&events, 3, 19.75).seconds, 1);
    let matches = formed(&run(&mut mm, 20.0, 20.0));
    assert_eq!(accounts(&matches[0].1), [1, 2, 3]);
}

#[test]
fn each_party_that_arrives_adds_five_seconds_up_to_forty() {
    let mut mm = matchmaker();
    mm.search(solo(1, BIG_TEAM, 5), 0.0).unwrap();
    mm.search(solo(2, BIG_TEAM, 5), 0.0).unwrap();
    let mut events = run(&mut mm, 0.0, 0.75);
    assert_eq!(status_at(&events, 1, 0.0).seconds, 20);
    for id in 3..=8 {
        let t = (id - 2) as f64;
        mm.search(solo(id, BIG_TEAM, 5), t).unwrap();
        events.extend(run(&mut mm, t, t + 0.75));
    }
    // 25 s once the third arrives at 1 s, 30 once the fourth does at 2 s,
    // and so on, but never past 40 s from the start.
    assert_eq!(status_at(&events, 1, 1.0).seconds, 24);
    assert_eq!(status_at(&events, 1, 2.0).seconds, 28);
    assert_eq!(status_at(&events, 1, 3.0).seconds, 32);
    assert_eq!(status_at(&events, 8, 6.75).seconds, 34);
    events.extend(run(&mut mm, 7.0, 39.75));
    assert!(formed(&events).is_empty());
    let matches = formed(&run(&mut mm, 40.0, 40.0));
    assert_eq!(matches[0].1.players.len(), 8);
    // Filled up to 12 with bots.
    assert_eq!(matches[0].1.bots, 4);
}

#[test]
fn a_full_match_starts_at_once() {
    let mut mm = matchmaker();
    for id in 1..=3 {
        mm.search(solo(id, RUMBLE_PIT, 5), 0.0).unwrap();
    }
    assert!(formed(&run(&mut mm, 0.0, 4.75)).is_empty());
    for id in 4..=8 {
        mm.search(solo(id, RUMBLE_PIT, 5), 5.0).unwrap();
    }
    let matches = formed(&run(&mut mm, 5.0, 5.0));
    assert_eq!(accounts(&matches[0].1), [1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn parties_only_meet_in_the_same_playlist_with_room_for_them() {
    let mut mm = matchmaker();
    mm.search(solo(1, HEAD_TO_HEAD, 5), 0.0).unwrap();
    mm.search(solo(2, RUMBLE_PIT, 5), 0.0).unwrap();
    let events = run(&mut mm, 0.0, 5.0);
    assert!(formed(&events).is_empty());
    assert_eq!(status_at(&events, 1, 5.0).stage, Stage::Searching);
    assert_eq!(status_at(&events, 2, 5.0).stage, Stage::Searching);

    // Four, three and two are too many for one game of eight, so the two
    // play apart.
    let mut mm = matchmaker();
    mm.search(party(10, TEAM_TRAINING, players(1, 4)), 0.0)
        .unwrap();
    mm.search(party(11, TEAM_TRAINING, players(5, 3)), 0.0)
        .unwrap();
    mm.search(party(12, TEAM_TRAINING, players(8, 2)), 0.0)
        .unwrap();
    let matches = formed(&run(&mut mm, 0.0, 20.0));
    let sizes: Vec<usize> = matches.iter().map(|(_, m)| m.players.len()).collect();
    assert_eq!(sizes, [7, 2]);
}

#[test]
fn level_ranges_widen_the_longer_a_match_gathers() {
    // Levels 1 and 20 are 19 apart: level 1's range of 10, widened by 9
    // after 45 s.
    let mut mm = matchmaker();
    mm.search(solo(1, HEAD_TO_HEAD, 1), 0.0).unwrap();
    mm.search(solo(2, HEAD_TO_HEAD, 20), 0.0).unwrap();
    let events = run(&mut mm, 0.0, 44.75);
    assert!(formed(&events).is_empty());
    let span = |t| {
        let s = status_at(&events, 1, t);
        (s.low, s.high)
    };
    assert_eq!(status_at(&events, 1, 0.0).stage, Stage::Searching);
    assert_eq!(span(0.0), (1, 11));
    assert_eq!(span(14.75), (1, 11));
    assert_eq!(span(15.0), (1, 14));
    assert_eq!(span(30.0), (1, 17));
    assert_eq!(span(44.75), (1, 17));
    let level_20 = status_at(&events, 2, 0.0);
    assert_eq!((level_20.low, level_20.high), (13, 28));
    assert_eq!(formed(&run(&mut mm, 45.0, 45.0)).len(), 1);

    // Levels 1 and 40 only meet once levels stop mattering, after a
    // minute.
    let mut mm = matchmaker();
    mm.search(solo(1, HEAD_TO_HEAD, 1), 0.0).unwrap();
    mm.search(solo(2, HEAD_TO_HEAD, 40), 0.0).unwrap();
    let events = run(&mut mm, 0.0, 59.75);
    assert!(formed(&events).is_empty());
    let level_40 = status_at(&events, 2, 0.0);
    assert_eq!((level_40.low, level_40.high), (30, 50));
    assert_eq!(formed(&run(&mut mm, 60.0, 60.0)).len(), 1);
}

#[test]
fn unranked_playlists_ignore_levels() {
    let mut mm = matchmaker();
    mm.search(solo(1, BIG_TEAM, 1), 0.0).unwrap();
    mm.search(solo(2, BIG_TEAM, 50), 0.0).unwrap();
    let s = status_at(&run(&mut mm, 0.0, 0.0), 1, 0.0);
    assert_eq!(
        (s.stage, s.have, s.low, s.high),
        (Stage::WaitingToFill, 2, 1, 50)
    );
    let m = &formed(&run(&mut mm, 0.25, 20.0))[0].1;
    assert!(!m.ranked);
    // Their level there is their best, to even out teams.
    assert_eq!(m.players[1].level, 50);
    assert_ne!(m.players[0].team, m.players[1].team);
}

#[test]
fn low_party_members_count_as_the_lowest_level_the_highest_can_meet() {
    // A level 12 meets levels 6 and up, so their level 1 friend counts as 6.
    let mut mm = matchmaker();
    let friends = || party(10, DOUBLE_TEAM, vec![member(1, 12), member(2, 1)]);
    mm.search(friends(), 0.0).unwrap();
    mm.search(solo(3, DOUBLE_TEAM, 6), 0.0).unwrap();
    mm.search(solo(4, DOUBLE_TEAM, 6), 0.0).unwrap();
    let m = &formed(&run(&mut mm, 0.0, 0.0))[0].1;
    let seats: Vec<(u64, u8, u8)> = m
        .players
        .iter()
        .map(|p| (p.account, p.level, p.effective))
        .collect();
    assert_eq!(seats, [(1, 12, 12), (2, 1, 6), (3, 6, 6), (4, 6, 6)]);
    assert_eq!(team(m, 1), team(m, 2));
    assert_eq!(team(m, 3), team(m, 4));
    assert_ne!(team(m, 1), team(m, 3));

    // A level 5 is too far below the level 12 until the range widens.
    let mut mm = matchmaker();
    mm.search(friends(), 0.0).unwrap();
    mm.search(solo(3, DOUBLE_TEAM, 5), 0.0).unwrap();
    mm.search(solo(4, DOUBLE_TEAM, 6), 0.0).unwrap();
    assert!(formed(&run(&mut mm, 0.0, 14.75)).is_empty());
    assert_eq!(formed(&run(&mut mm, 15.0, 15.0)).len(), 1);
}

#[test]
fn parties_too_big_for_the_playlist_are_refused() {
    let mut mm = matchmaker();
    assert_eq!(
        mm.search(party(10, RUMBLE_PIT, players(1, 2)), 0.0),
        Err(TOO_LARGE)
    );
    assert_eq!(
        mm.search(party(10, HEAD_TO_HEAD, players(1, 2)), 0.0),
        Err(TOO_LARGE)
    );
    assert_eq!(
        mm.search(party(10, DOUBLE_TEAM, players(1, 2)), 0.0),
        Ok(DOUBLE_TEAM)
    );
    assert_eq!(
        mm.search(party(11, TEAM_SLAYER, players(1, 5)), 0.0),
        Err(TOO_LARGE)
    );
    assert_eq!(
        mm.search(party(11, TEAM_SLAYER, players(1, 4)), 0.0),
        Ok(TEAM_SLAYER)
    );
    assert_eq!(mm.search(party(12, 77, players(1, 2)), 0.0), Err(INVALID));
    assert_eq!(
        mm.search(party(13, RUMBLE_PIT, Vec::new()), 0.0),
        Err(INVALID)
    );
    // A ranked team playlist with room for a party it could never put
    // against a team one smaller.
    let lopsided = parse(
        "playlist 0 lopsided Lopsided\nranked yes\nhumans 4 8\nbots even\n\
         variant team_slayer default 50 0\nmaps lockout\n",
    );
    let mut mm = Matchmaker::new(lopsided.unwrap(), 1);
    assert_eq!(mm.search(party(1, 0, players(1, 6)), 0.0), Err(TOO_LARGE));
    assert_eq!(mm.search(party(2, 0, players(1, 4)), 0.0), Ok(0));
}

#[test]
fn guests_play_only_where_the_playlist_allows() {
    let with_guests = |guests| Member {
        guests,
        ..member(1, 5)
    };
    let mut mm = matchmaker();
    let ranked = party(10, DOUBLE_TEAM, vec![with_guests(1)]);
    assert_eq!(mm.search(ranked, 0.0), Err(NO_RANKED_GUESTS));
    assert_eq!(
        mm.search(party(10, BIG_TEAM, vec![with_guests(3)]), 0.0),
        Ok(BIG_TEAM)
    );
    mm.search(solo(2, BIG_TEAM, 5), 0.0).unwrap();
    assert_eq!(mm.searching(BIG_TEAM), 5);
    let m = &formed(&run(&mut mm, 0.0, 20.0))[0].1;
    assert_eq!(m.players[0].guests, 3);
    // Five people, filled up to 12; the guests stay on their PC's team.
    assert_eq!(m.bots, 7);
    assert_ne!(team(m, 1), team(m, 2));

    let no_guests =
        parse("playlist 0 x X\nhumans 2 4\nvariant slayer default 10 0\nmaps lockout\n");
    let mut mm = Matchmaker::new(no_guests.unwrap(), 1);
    assert_eq!(
        mm.search(party(1, 0, vec![with_guests(1)]), 0.0),
        Err(NO_GUESTS)
    );
}

#[test]
fn everyone_needs_the_same_copy_of_a_map() {
    let with_maps = |id: u64, maps: &[(&str, u64)]| {
        let maps = maps.iter().map(|&(n, h)| (n.to_string(), h)).collect();
        party(
            id,
            HEAD_TO_HEAD,
            vec![Member {
                maps,
                ..member(id, 5)
            }],
        )
    };
    let mut mm = matchmaker();
    mm.search(with_maps(1, &[("lockout", 7)]), 0.0).unwrap();
    mm.search(with_maps(2, &[("midship", 7)]), 0.0).unwrap();
    mm.search(with_maps(3, &[("lockout", 8)]), 0.0).unwrap();
    assert!(formed(&run(&mut mm, 0.0, 120.0)).is_empty());
    // Lockout, the same file as the first player's.
    mm.search(with_maps(4, &[("LOCKOUT", 7), ("midship", 9)]), 120.0)
        .unwrap();
    let m = &formed(&run(&mut mm, 120.25, 120.25))[0].1;
    assert_eq!(accounts(m), [1, 4]);
    assert_eq!((m.map.as_str(), m.hash), ("lockout", 7));

    // A party needs a map all its members have.
    let a = Member {
        maps: vec![("lockout".into(), 7)],
        ..member(5, 5)
    };
    let b = Member {
        maps: vec![("lockout".into(), 8), ("midship".into(), 7)],
        ..member(6, 5)
    };
    assert_eq!(
        mm.search(party(20, DOUBLE_TEAM, vec![a, b]), 0.0),
        Err(NO_MAPS)
    );
    assert_eq!(
        mm.search(with_maps(21, &[("zanzibar", 7)]), 0.0),
        Err(NO_MAPS)
    );
}

#[test]
fn no_one_is_matched_on_a_map_they_dont_have() {
    // Four search Team Slayer: one without Lockout, one with another copy
    // of Midship. The game wouldn't let them, but whoever searches anyway
    // only plays the maps everyone has.
    let mut maps = HashSet::new();
    for seed in 0..40 {
        let mut mm = Matchmaker::new(built_in(), seed);
        let mut without = member(1, 5);
        without.maps.retain(|(m, _)| m != "lockout");
        let mut other_copy = member(2, 5);
        for (m, hash) in &mut other_copy.maps {
            if m == "midship" {
                *hash = 8;
            }
        }
        mm.search(party(1, TEAM_SLAYER, vec![without]), 0.0)
            .unwrap();
        mm.search(party(2, TEAM_SLAYER, vec![other_copy]), 0.0)
            .unwrap();
        mm.search(solo(3, TEAM_SLAYER, 5), 0.0).unwrap();
        mm.search(solo(4, TEAM_SLAYER, 5), 0.0).unwrap();
        let m = &formed(&run(&mut mm, 0.0, 30.0))[0].1;
        assert_eq!(accounts(m), [1, 2, 3, 4]);
        assert!(
            !["lockout", "midship"].contains(&m.map.as_str()),
            "{}",
            m.map
        );
        assert_eq!(m.hash, 7);
        maps.insert(m.map.clone());
    }
    // Any of the other eight.
    assert!(maps.len() > 4, "{maps:?}");

    // Quickmatch passes over playlists the party lacks a map of, as the
    // game marks them: Rumble Pit is busiest, but plays Warlock.
    let mut mm = matchmaker();
    for id in 1..=6 {
        mm.search(solo(id, RUMBLE_PIT, 5), 0.0).unwrap();
    }
    mm.search(solo(7, TEAM_SLAYER, 5), 0.0).unwrap();
    let mut no_warlock = solo(10, QUICKMATCH, 5);
    no_warlock.members[0].maps.retain(|(m, _)| m != "warlock");
    assert_eq!(mm.search(no_warlock, 0.0), Ok(TEAM_SLAYER));
    // With Lockout alone, no playlist has every map, and Quickmatch says
    // so, though Head to Head can still be searched by name.
    let mut only_lockout = solo(20, QUICKMATCH, 5);
    only_lockout.members[0].maps = vec![("lockout".into(), 7)];
    assert_eq!(mm.search(only_lockout.clone(), 0.0), Err(MISSING_CONTENT));
    only_lockout.playlist = HEAD_TO_HEAD;
    assert_eq!(mm.search(only_lockout, 0.0), Ok(HEAD_TO_HEAD));
}

#[test]
fn a_match_is_never_on_the_map_a_party_just_played() {
    let mut maps = HashSet::new();
    for seed in 0..20 {
        let mut mm = Matchmaker::new(built_in(), seed);
        let mut first = solo(1, HEAD_TO_HEAD, 5);
        first.previous_map = Some("lockout".into());
        mm.search(first, 0.0).unwrap();
        mm.search(solo(2, HEAD_TO_HEAD, 5), 0.0).unwrap();
        let m = &formed(&run(&mut mm, 0.0, 0.0))[0].1;
        assert_ne!(m.map, "lockout");
        maps.insert(m.map.clone());
    }
    // The rest are picked at random.
    assert!(maps.len() > 2);

    // Unless it's the only one everyone has.
    let only_lockout = |id| {
        let mut ticket = solo(id, HEAD_TO_HEAD, 5);
        ticket.members[0].maps = vec![("lockout".into(), 7)];
        ticket.previous_map = Some("lockout".into());
        ticket
    };
    let mut mm = matchmaker();
    mm.search(only_lockout(1), 0.0).unwrap();
    mm.search(only_lockout(2), 0.0).unwrap();
    assert_eq!(formed(&run(&mut mm, 0.0, 0.0))[0].1.map, "lockout");
}

#[test]
fn matchmaking_fails_after_ten_minutes() {
    let mut mm = matchmaker();
    mm.search(solo(1, RUMBLE_PIT, 5), 0.0).unwrap();
    let events = run(&mut mm, 0.0, 600.0);
    let failed: Vec<f64> = events
        .iter()
        .filter(|(_, e)| *e == Event::Failed { party: 1 })
        .map(|(t, _)| *t)
        .collect();
    assert_eq!(failed, [600.0]);
    let last = status_at(&events, 1, 600.0);
    assert_eq!(
        last,
        Status {
            stage: Stage::Searching,
            have: 1,
            need: 2,
            seconds: 599,
            low: 1,
            high: 50,
        }
    );
    assert_eq!(mm.searching(RUMBLE_PIT), 0);
    assert!(run(&mut mm, 600.25, 700.0).is_empty());
}

#[test]
fn ranked_teams_wait_for_even_sides() {
    let mut mm = matchmaker();
    mm.search(party(10, TEAM_SLAYER, players(1, 4)), 0.0)
        .unwrap();
    let events = run(&mut mm, 0.0, 0.75);
    // Four against no one: three more make it four against three and a bot.
    let s = status_at(&events, 10, 0.0);
    assert_eq!((s.stage, s.have, s.need), (Stage::Balancing, 4, 3));
    mm.search(party(11, TEAM_SLAYER, players(5, 3)), 1.0)
        .unwrap();
    let events = run(&mut mm, 1.0, 20.75);
    assert_eq!(status_at(&events, 11, 1.0).stage, Stage::WaitingToFill);
    assert!(formed(&events).is_empty());
    let m = &formed(&run(&mut mm, 21.0, 21.0))[0].1;
    assert_eq!(m.bots, 1);
    assert!((2..=4).all(|a| team(m, a) == team(m, 1)));
    assert!((5..=7).all(|a| team(m, a) != team(m, 1)));
}

#[test]
fn double_team_is_two_against_two() {
    let mut mm = matchmaker();
    mm.search(party(10, DOUBLE_TEAM, players(1, 2)), 0.0)
        .unwrap();
    mm.search(solo(3, DOUBLE_TEAM, 5), 0.0).unwrap();
    let events = run(&mut mm, 0.0, 1.0);
    let s = status_at(&events, 10, 0.0);
    assert_eq!((s.stage, s.have, s.need), (Stage::Gathering, 3, 1));
    mm.search(solo(4, DOUBLE_TEAM, 5), 1.0).unwrap();
    let m = &formed(&run(&mut mm, 1.25, 1.25))[0].1;
    assert_eq!(m.bots, 0);
    assert_eq!(team(m, 1), team(m, 2));
    assert_eq!(team(m, 3), team(m, 4));
    assert_ne!(team(m, 1), team(m, 3));
}

#[test]
fn a_party_that_would_leave_teams_uneven_waits_for_another_match() {
    // Three, three and two can't make teams one apart.
    let mut mm = matchmaker();
    mm.search(party(10, TEAM_SLAYER, players(1, 3)), 0.0)
        .unwrap();
    mm.search(party(11, TEAM_SLAYER, players(4, 3)), 0.0)
        .unwrap();
    mm.search(party(12, TEAM_SLAYER, players(7, 2)), 0.0)
        .unwrap();
    let events = run(&mut mm, 0.0, 20.0);
    let matches = formed(&events);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].0, 20.0);
    assert_eq!(accounts(&matches[0].1), [1, 2, 3, 4, 5, 6]);
    assert_eq!(matches[0].1.bots, 0);
    let s = status_at(&events, 12, 20.0);
    assert_eq!((s.stage, s.have, s.need), (Stage::Searching, 2, 2));
    assert_eq!(mm.searching(TEAM_SLAYER), 2);
}

#[test]
fn a_party_that_would_unbalance_a_ready_match_waits_for_the_next() {
    // Three pairs: two play two against two, and the third waits rather
    // than leave them all a player short.
    let mut mm = matchmaker();
    for (id, from) in [(10, 1), (11, 3), (12, 5)] {
        mm.search(party(id, TEAM_SKIRMISH, players(from, 2)), 0.0)
            .unwrap();
    }
    let events = run(&mut mm, 0.0, 20.0);
    let s = status_at(&events, 10, 0.0);
    assert_eq!((s.stage, s.have, s.need), (Stage::WaitingToFill, 4, 0));
    let s = status_at(&events, 12, 0.0);
    assert_eq!((s.stage, s.have, s.need), (Stage::Searching, 2, 2));
    let matches = formed(&events);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].0, 20.0);
    assert_eq!(accounts(&matches[0].1), [1, 2, 3, 4]);
    assert_eq!(mm.searching(TEAM_SKIRMISH), 2);

    // The same when the third pair arrives during the countdown.
    let mut mm = matchmaker();
    mm.search(party(10, TEAM_SLAYER, players(1, 2)), 0.0)
        .unwrap();
    mm.search(party(11, TEAM_SLAYER, players(3, 2)), 0.0)
        .unwrap();
    let mut events = run(&mut mm, 0.0, 14.75);
    mm.search(party(12, TEAM_SLAYER, players(5, 2)), 15.0)
        .unwrap();
    events.extend(run(&mut mm, 15.0, 20.0));
    let s = status_at(&events, 10, 15.25);
    assert_eq!((s.stage, s.have, s.seconds), (Stage::WaitingToFill, 4, 5));
    let matches = formed(&events);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].0, 20.0);
    assert_eq!(accounts(&matches[0].1), [1, 2, 3, 4]);

    // But a fourth pair makes four against four, at once.
    let mut mm = matchmaker();
    for (id, from) in [(10, 1), (11, 3), (12, 5)] {
        mm.search(party(id, TEAM_SLAYER, players(from, 2)), 0.0)
            .unwrap();
    }
    assert!(formed(&run(&mut mm, 0.0, 4.75)).is_empty());
    mm.search(party(13, TEAM_SLAYER, players(7, 2)), 5.0)
        .unwrap();
    let m = &formed(&run(&mut mm, 5.0, 5.0))[0].1;
    assert_eq!(accounts(m), [1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(m.bots, 0);
    assert!((1..=7).step_by(2).all(|a| team(m, a) == team(m, a + 1)));
}

#[test]
fn without_bots_ranked_teams_must_be_the_same_size() {
    let no_bots = parse(
        "playlist 0 even Even Teams\nranked yes\nhumans 4 8\n\
         variant team_slayer default 50 0\nmaps lockout\n",
    );
    let mut mm = Matchmaker::new(no_bots.unwrap(), 1);
    mm.search(party(10, 0, players(1, 3)), 0.0).unwrap();
    mm.search(party(11, 0, players(4, 2)), 0.0).unwrap();
    let events = run(&mut mm, 0.0, 0.75);
    let s = status_at(&events, 10, 0.0);
    assert_eq!((s.stage, s.have, s.need), (Stage::Balancing, 5, 1));
    mm.search(solo(7, 0, 5), 1.0).unwrap();
    let events = run(&mut mm, 1.0, 21.0);
    let m = &formed(&events)[0].1;
    assert_eq!(m.bots, 0);
    assert!([2, 3].iter().all(|&a| team(m, a) == team(m, 1)));
    assert!([4, 5, 7].iter().all(|&a| team(m, a) != team(m, 1)));
}

#[test]
fn a_match_counting_down_doesnt_give_up() {
    let mut mm = matchmaker();
    mm.search(solo(1, RUMBLE_PIT, 5), 0.0).unwrap();
    let mut events = run(&mut mm, 0.0, 589.75);
    for id in 2..=3 {
        mm.search(solo(id, RUMBLE_PIT, 5), 590.0).unwrap();
    }
    events.extend(run(&mut mm, 590.0, 610.0));
    assert!(!events
        .iter()
        .any(|(_, e)| matches!(e, Event::Failed { .. })));
    let matches = formed(&events);
    assert_eq!(matches[0].0, 610.0);
    assert_eq!(accounts(&matches[0].1), [1, 2, 3]);
}

#[test]
fn unranked_matches_fill_up_with_bots() {
    let mut mm = matchmaker();
    for id in 1..=3 {
        mm.search(solo(id, TEAM_TRAINING, 5), 0.0).unwrap();
    }
    let m = &formed(&run(&mut mm, 0.0, 20.0))[0].1;
    assert_eq!(m.bots, 5);
    let red = m.players.iter().filter(|p| p.team == 0).count();
    assert!(red == 1 || red == 2);
}

#[test]
fn team_snipers_is_unranked_team_slayer_with_sniper_rifles() {
    // Two are enough: bots make it four against four. Guests come too.
    let mut mm = matchmaker();
    let host = Member {
        guests: 1,
        ..member(1, 5)
    };
    assert_eq!(
        mm.search(party(1, TEAM_SNIPERS, vec![host]), 0.0),
        Ok(TEAM_SNIPERS)
    );
    mm.search(solo(2, TEAM_SNIPERS, 30), 0.0).unwrap();
    let m = &formed(&run(&mut mm, 0.0, 30.0))[0].1;
    assert_eq!((m.playlist, m.ranked, m.bots), (TEAM_SNIPERS, false, 5));
    assert_eq!(m.variant.game_type, GameType::TeamSlayer);
    assert_eq!(m.variant.preset, "TEAM SNIPERS");
    assert_ne!(team(m, 1), team(m, 2));
    // Parties of up to four, so teams can always be even.
    let five = party(10, TEAM_SNIPERS, players(10, 5));
    assert_eq!(mm.search(five, 0.0), Err(TOO_LARGE));
}

#[test]
fn team_hardcore_is_ranked_four_against_four_in_its_own_variants() {
    let mut mm = matchmaker();
    let ranked = party(
        10,
        TEAM_HARDCORE,
        vec![Member {
            guests: 1,
            ..member(10, 5)
        }],
    );
    assert_eq!(mm.search(ranked, 0.0), Err(NO_RANKED_GUESTS));
    mm.search(party(1, TEAM_HARDCORE, players(1, 4)), 0.0)
        .unwrap();
    for id in 5..=8 {
        mm.search(solo(id, TEAM_HARDCORE, 5), 0.0).unwrap();
    }
    let m = &formed(&run(&mut mm, 0.0, 0.0))[0].1;
    assert_eq!((m.playlist, m.ranked, m.bots), (TEAM_HARDCORE, true, 0));
    assert_eq!(m.players.len(), 8);
    // The party of four stays together, against the four alone.
    assert!((1..=4).all(|a| team(m, a) == team(m, 1)));
    assert!((5..=8).all(|a| team(m, a) != team(m, 1)));
    let playlist = &built_in()[usize::from(TEAM_HARDCORE)];
    assert!(playlist.variants.contains(&m.variant));
    assert!(playlist.maps.contains(&m.map));
    assert!(["HARDCORE", "TEAM SNIPERS"].contains(&m.variant.preset.as_str()));
}

#[test]
fn unranked_teams_are_even_once_bots_join_the_smaller() {
    // Team Training is four against four at most, so a party of more than
    // four could never have enough opponents.
    let mut mm = matchmaker();
    for n in [5, 6, 8] {
        let big = party(10, TEAM_TRAINING, players(1, n));
        assert_eq!(mm.search(big, 0.0), Err(TOO_LARGE));
    }
    // Four play four bots.
    mm.search(party(10, TEAM_TRAINING, players(1, 4)), 0.0)
        .unwrap();
    let m = &formed(&run(&mut mm, 0.0, 20.0))[0].1;
    assert_eq!(m.bots, 4);
    assert!(m.players.iter().all(|p| p.team == m.players[0].team));

    // Three, three and two can't be closer than five against three, so the
    // two play apart.
    let mut mm = matchmaker();
    mm.search(party(10, TEAM_TRAINING, players(1, 3)), 0.0)
        .unwrap();
    mm.search(party(11, TEAM_TRAINING, players(4, 3)), 0.0)
        .unwrap();
    mm.search(party(12, TEAM_TRAINING, players(7, 2)), 0.0)
        .unwrap();
    let matches = formed(&run(&mut mm, 0.0, 20.0));
    let sizes: Vec<(usize, u8)> = matches
        .iter()
        .map(|(_, m)| (m.players.len(), m.bots))
        .collect();
    assert_eq!(sizes, [(6, 2), (2, 6)]);

    // Six and two in Big Team Battle: the four bots it fills up with join
    // the two.
    let mut mm = matchmaker();
    mm.search(party(10, BIG_TEAM, players(1, 6)), 0.0).unwrap();
    mm.search(party(11, BIG_TEAM, players(7, 2)), 0.0).unwrap();
    let m = &formed(&run(&mut mm, 0.0, 20.0))[0].1;
    assert_eq!(m.bots, 4);
    assert_ne!(team(m, 1), team(m, 7));

    // A party of eight there waits for seven more to play against.
    let mut mm = matchmaker();
    mm.search(party(10, BIG_TEAM, players(1, 8)), 0.0).unwrap();
    let events = run(&mut mm, 0.0, 30.0);
    assert!(formed(&events).is_empty());
    let s = status_at(&events, 10, 30.0);
    assert_eq!((s.stage, s.have, s.need), (Stage::Balancing, 8, 7));
}

#[test]
fn quickmatch_picks_the_busiest_playlist_the_party_fits() {
    let mut mm = matchmaker();
    for id in 1..=6 {
        mm.search(solo(id, RUMBLE_PIT, 5), 0.0).unwrap();
    }
    for id in 7..=9 {
        mm.search(solo(id, TEAM_SLAYER, 5), 0.0).unwrap();
    }
    // Rumble Pit is for players alone.
    let pair = party(20, QUICKMATCH, players(20, 2));
    assert_eq!(mm.search(pair, 0.0), Ok(TEAM_SLAYER));
    assert_eq!(mm.searching(TEAM_SLAYER), 5);
    assert_eq!(mm.search(solo(30, QUICKMATCH, 5), 0.0), Ok(RUMBLE_PIT));
    // Guests only fit the unranked playlists, where no one's searching yet:
    // any will do.
    let mut picked = HashSet::new();
    for seed in 0..20 {
        let mut mm = Matchmaker::new(built_in(), seed);
        let host = Member {
            guests: 1,
            ..member(40, 5)
        };
        picked.insert(mm.search(party(40, QUICKMATCH, vec![host]), 0.0).unwrap());
    }
    assert_eq!(
        picked,
        HashSet::from([BIG_TEAM, TEAM_TRAINING, TEAM_SNIPERS])
    );
    let nine = party(50, QUICKMATCH, players(50, 9));
    assert_eq!(mm.search(nine, 0.0), Err(NO_PLAYLIST));
}

#[test]
fn the_games_players_and_the_launchers_never_meet() {
    let mut mm = matchmaker();
    // Each can search only its own playlists.
    let launcher = |id: u64, playlist: u8| launchers(id, playlist, vec![member(id, 5)]);
    assert_eq!(mm.search(launcher(1, HEAD_TO_HEAD), 0.0), Err(INVALID));
    assert_eq!(mm.search(solo(2, MCC_HEAD_TO_HEAD, 5), 0.0), Err(INVALID));
    // So one of each searching Head to Head never makes a match...
    mm.search(launcher(1, MCC_HEAD_TO_HEAD), 0.0).unwrap();
    mm.search(solo(2, HEAD_TO_HEAD, 5), 0.0).unwrap();
    assert!(formed(&run(&mut mm, 0.0, 30.0)).is_empty());
    // ...but another of either does, with its own kind.
    mm.search(launcher(3, MCC_HEAD_TO_HEAD), 30.0).unwrap();
    mm.search(solo(4, HEAD_TO_HEAD, 5), 30.0).unwrap();
    let mut made: Vec<(u8, Vec<u64>)> = formed(&run(&mut mm, 30.0, 31.0))
        .into_iter()
        .map(|(_, m)| (m.playlist, accounts(&m)))
        .collect();
    made.sort();
    assert_eq!(made, [(HEAD_TO_HEAD, vec![2, 4]), (MCC_HEAD_TO_HEAD, vec![1, 3])]);
    // Quickmatch picks among the party's own kind of playlist only.
    for seed in 0..20 {
        let mut mm = Matchmaker::new(built_in(), seed);
        for id in 1..=6 {
            mm.search(solo(id, RUMBLE_PIT, 5), 0.0).unwrap();
        }
        let picked = mm.search(launcher(9, QUICKMATCH), 0.0).unwrap();
        let playlist = built_in().into_iter().find(|p| p.id == picked).unwrap();
        assert_eq!(playlist.client, ClientKind::Launcher, "{picked}");
        let picked = mm.search(solo(10, QUICKMATCH, 5), 0.0).unwrap();
        assert_eq!(picked, RUMBLE_PIT);
    }
    // A launcher party searching with the launcher's Rumble Pit busy goes
    // there.
    let mut mm = matchmaker();
    for id in 1..=2 {
        mm.search(launcher(id, MCC_RUMBLE_PIT), 0.0).unwrap();
    }
    assert_eq!(mm.search(launcher(5, QUICKMATCH), 0.0), Ok(MCC_RUMBLE_PIT));
    // And a party of four where launchers play in fours.
    mm.search(launchers(6, MCC_TEAM_SLAYER, players(6, 2)), 0.0)
        .unwrap();
    let four = launchers(10, QUICKMATCH, players(10, 4));
    assert_eq!(mm.search(four, 0.0), Ok(MCC_TEAM_SLAYER));
}

#[test]
fn launcher_matches_play_mcc_variants() {
    let mut mm = matchmaker();
    for id in 1..=2 {
        let ticket = launchers(id, MCC_HEAD_TO_HEAD, vec![member(id, 5)]);
        mm.search(ticket, 0.0).unwrap();
    }
    let made = formed(&run(&mut mm, 0.0, 1.0));
    let [(_, m)] = &made[..] else {
        panic!("{made:?}");
    };
    assert_eq!(m.variant.mcc.as_deref(), Some("01_slayer"));
    assert_eq!((m.variant.game_type, m.bots), (GameType::Slayer, 0));
    let playlist = built_in().into_iter().find(|p| p.id == MCC_HEAD_TO_HEAD);
    assert!(playlist.unwrap().maps.contains(&m.map));
}

#[test]
fn a_new_search_replaces_the_last() {
    let mut mm = matchmaker();
    mm.search(solo(1, RUMBLE_PIT, 5), 0.0).unwrap();
    mm.search(solo(1, HEAD_TO_HEAD, 5), 1.0).unwrap();
    assert_eq!(mm.searching(RUMBLE_PIT), 0);
    assert_eq!(mm.searching(HEAD_TO_HEAD), 1);
    // A refused search still ends the last one.
    assert_eq!(
        mm.search(party(1, HEAD_TO_HEAD, players(1, 2)), 2.0),
        Err(TOO_LARGE)
    );
    assert_eq!(mm.searching(HEAD_TO_HEAD), 0);
}

#[test]
fn a_party_that_cancels_leaves_the_match_it_was_in() {
    let mut mm = matchmaker();
    for id in 1..=3 {
        mm.search(solo(id, RUMBLE_PIT, 5), 0.0).unwrap();
    }
    let mut events = run(&mut mm, 0.0, 9.75);
    assert!(mm.cancel(2));
    assert!(!mm.cancel(2));
    events.extend(run(&mut mm, 10.0, 11.75));
    let s = status_at(&events, 1, 10.0);
    assert_eq!((s.stage, s.have, s.need), (Stage::Gathering, 2, 1));
    // Back to its fewest: the countdown starts again from 20 s.
    mm.search(solo(4, RUMBLE_PIT, 5), 12.0).unwrap();
    events.extend(run(&mut mm, 12.0, 31.75));
    assert!(formed(&events).is_empty());
    assert_eq!(status_at(&events, 4, 12.0).seconds, 20);
    let m = &formed(&run(&mut mm, 32.0, 32.0))[0].1;
    assert_eq!(accounts(m), [1, 3, 4]);
}

#[test]
fn a_countdown_carries_on_when_its_oldest_party_leaves() {
    let mut mm = matchmaker();
    for id in 1..=4 {
        mm.search(solo(id, RUMBLE_PIT, 5), 0.0).unwrap();
    }
    let mut events = run(&mut mm, 0.0, 14.75);
    mm.cancel(1);
    events.extend(run(&mut mm, 15.0, 19.75));
    assert_eq!(status_at(&events, 2, 15.25).seconds, 5);
    assert!(formed(&events).is_empty());
    let m = &formed(&run(&mut mm, 20.0, 20.0))[0].1;
    assert_eq!(accounts(m), [2, 3, 4]);
}

#[test]
fn a_party_that_takes_over_a_countdown_arrives_like_any_other() {
    // A level 30 searching for a minute stops minding levels and takes in
    // three level 1s counting down since 50 s: five seconds more, not a
    // fresh twenty.
    let mut mm = matchmaker();
    mm.search(solo(1, RUMBLE_PIT, 30), 0.0).unwrap();
    let mut events = run(&mut mm, 0.0, 49.75);
    for id in 2..=4 {
        mm.search(solo(id, RUMBLE_PIT, 1), 50.0).unwrap();
    }
    events.extend(run(&mut mm, 50.0, 74.75));
    assert_eq!(status_at(&events, 2, 59.75).seconds, 11);
    assert_eq!(status_at(&events, 2, 60.0).seconds, 15);
    let s = status_at(&events, 1, 60.0);
    assert_eq!((s.stage, s.have, s.seconds), (Stage::WaitingToFill, 4, 15));
    assert!(formed(&events).is_empty());
    let m = &formed(&run(&mut mm, 75.0, 75.0))[0].1;
    assert_eq!(accounts(m), [1, 2, 3, 4]);
}

#[test]
fn statuses_are_sent_when_they_change() {
    let mut mm = matchmaker();
    mm.search(solo(1, RUMBLE_PIT, 10), 0.0).unwrap();
    let shown = statuses(&run(&mut mm, 0.0, 3.75), 1);
    let seconds: Vec<(f64, u16)> = shown.iter().map(|(t, s)| (*t, s.seconds)).collect();
    assert_eq!(seconds, [(0.0, 0), (1.0, 1), (2.0, 2), (3.0, 3)]);
    let searching = Status {
        stage: Stage::Searching,
        have: 1,
        need: 2,
        seconds: 0,
        low: 1,
        high: 17,
    };
    assert_eq!(shown[0].1, searching);
}

#[test]
fn teams_keep_parties_whole_and_even_out_levels() {
    let mut rng = Rng::new(1);
    // Four players alone: 10 and 1 against 9 and 2.
    let teams = split(&[1, 1, 1, 1], &[10, 9, 2, 1], &mut rng);
    assert_eq!(teams[0], teams[3]);
    assert_eq!(teams[1], teams[2]);
    assert_ne!(teams[0], teams[1]);
    // A party of two plays the two alone, however strong they are.
    let teams = split(&[2, 1, 1], &[60, 20, 5], &mut rng);
    assert_eq!(teams[1], teams[2]);
    assert_ne!(teams[0], teams[1]);
    // A party of three (levels adding up to 30) takes the weakest of five
    // players alone: 31 against 34.
    let teams = split(&[3, 1, 1, 1, 1, 1], &[30, 10, 9, 8, 7, 1], &mut rng);
    assert_eq!(teams[0], teams[5]);
    assert!((1..5).all(|i| teams[i] != teams[0]));
    // Sixteen players alone: eight a side.
    let teams = split(&[1; 16], &[5; 16], &mut rng);
    assert_eq!(teams.iter().filter(|&&t| t == 0).count(), 8);
    // Equal splits are picked at random.
    let sides: HashSet<Vec<u8>> = (0..20).map(|_| split(&[1, 1], &[5, 5], &mut rng)).collect();
    assert_eq!(sides.len(), 2);
}

#[test]
fn the_fastest_pc_hosts_unless_a_bigger_party_is_nearly_as_fast() {
    let mut mm = matchmaker();
    mm.search(party(10, DOUBLE_TEAM, vec![pc(1, 40), pc(2, 45)]), 0.0)
        .unwrap();
    mm.search(party(3, DOUBLE_TEAM, vec![pc(3, 35)]), 0.0)
        .unwrap();
    mm.search(party(4, DOUBLE_TEAM, vec![pc(4, 100)]), 0.0)
        .unwrap();
    let m = formed(&run(&mut mm, 0.0, 0.0))[0].1.clone();
    assert_eq!(m.host, 1);
    // The others are asked in turn when the one asked doesn't start
    // hosting in time.
    let asked: Vec<(f64, Event)> = run(&mut mm, 0.25, 200.0);
    let id = m.id;
    assert_eq!(
        asked,
        [
            (45.0, Event::NewHost { id, host: 2 }),
            (90.0, Event::NewHost { id, host: 3 }),
            (135.0, Event::NewHost { id, host: 4 }),
            (180.0, Event::NoHost { id }),
        ]
    );

    // More than 10 ms slower loses, however big the party.
    let mut mm = matchmaker();
    mm.search(party(10, DOUBLE_TEAM, vec![pc(1, 50), pc(2, 60)]), 0.0)
        .unwrap();
    mm.search(party(3, DOUBLE_TEAM, vec![pc(3, 35)]), 0.0)
        .unwrap();
    mm.search(party(4, DOUBLE_TEAM, vec![pc(4, 100)]), 0.0)
        .unwrap();
    assert_eq!(formed(&run(&mut mm, 0.0, 0.0))[0].1.host, 3);
}

#[test]
fn pcs_with_no_round_trip_measured_yet_can_still_host() {
    // A PC that hasn't answered a ping yet counts as the slowest there is.
    let none = HashMap::new();
    assert_eq!(host_order(&[(1, u32::MAX, 1), (2, 20, 1)], &none), [2, 1]);
    let mut mm = matchmaker();
    mm.search(party(1, HEAD_TO_HEAD, vec![pc(1, u32::MAX)]), 0.0)
        .unwrap();
    mm.search(party(2, HEAD_TO_HEAD, vec![pc(2, u32::MAX)]), 0.0)
        .unwrap();
    let m = formed(&run(&mut mm, 0.0, 0.0))[0].1.clone();
    assert_eq!(m.host, 1);
    let next = Event::NewHost { id: m.id, host: 2 };
    assert_eq!(mm.host_failed(m.id, 1.0), Some(next));
}

#[test]
fn a_pc_that_left_its_hosted_match_hosts_last_for_three_matches() {
    let mut mm = matchmaker();
    mm.host_left(1);
    let mut hosts = Vec::new();
    for k in 0..4 {
        let t = k as f64 * 10.0;
        mm.search(party(1, HEAD_TO_HEAD, vec![pc(1, 10)]), t)
            .unwrap();
        mm.search(party(2, HEAD_TO_HEAD, vec![pc(2, 50)]), t)
            .unwrap();
        hosts.push(formed(&run(&mut mm, t, t))[0].1.host);
    }
    assert_eq!(hosts, [2, 2, 2, 1]);
}

#[test]
fn the_next_pc_is_asked_to_host_after_45_seconds() {
    let mut mm = matchmaker();
    let both = |mm: &mut Matchmaker, t| {
        mm.search(party(1, HEAD_TO_HEAD, vec![pc(1, 10)]), t)
            .unwrap();
        mm.search(party(2, HEAD_TO_HEAD, vec![pc(2, 50)]), t)
            .unwrap();
    };
    both(&mut mm, 0.0);
    let m = formed(&run(&mut mm, 0.0, 0.0))[0].1.clone();
    assert_eq!(m.host, 1);
    assert!(run(&mut mm, 0.25, 44.75).is_empty());
    let next = Event::NewHost { id: m.id, host: 2 };
    assert_eq!(run(&mut mm, 45.0, 45.0), [(45.0, next)]);
    // The first is too late now.
    assert!(!mm.hosting(m.id, 1));
    assert!(mm.hosting(m.id, 2));
    assert!(!mm.hosting(m.id, 2));
    assert!(run(&mut mm, 45.25, 200.0).is_empty());

    // A PC that can't host hands over at once.
    both(&mut mm, 300.0);
    let m = formed(&run(&mut mm, 300.0, 300.0))[0].1.clone();
    let next = Event::NewHost { id: m.id, host: 2 };
    assert_eq!(mm.host_failed(m.id, 301.0), Some(next));
    assert_eq!(
        mm.host_failed(m.id, 302.0),
        Some(Event::NoHost { id: m.id })
    );
    assert_eq!(mm.host_failed(m.id, 303.0), None);
    assert!(run(&mut mm, 300.25, 400.0).is_empty());
}
