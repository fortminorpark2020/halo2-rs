//! Friends lists and service records through the server: launchers asking,
//! answering and removing, what friends see of each other, the limits, and
//! that the game never hears of any of it.

use super::*;
use h2net::live::{
    is_friend_notice, Friend, LauncherMatch, LauncherPlayerResult, MatchOver, Online, Relation,
    ServiceRecord,
};

/// Head to Head, a ranked launcher playlist for two.
const HEAD_TO_HEAD: u8 = 11;
/// Maps every launcher here has (Head to Head plays all of them).
const MAPS: [&str; 3] = ["lockout", "midship", "warlock"];

/// A relay for the launchers' matches, kept for as long as it's returned.
fn relay(w: &mut World) -> h2relay::RelayServer {
    let config = h2relay::ServerConfig {
        admission: h2relay::Admission::Issued,
        ..Default::default()
    };
    let relay = h2relay::RelayServer::bind(("127.0.0.1", 0), config).unwrap();
    w.server.set_relay(relay.handle());
    relay
}

fn launcher(w: &mut World, n: u8, gamertag: &str) -> usize {
    sign_in_launcher(w, n, gamertag, &MAPS)
}

/// The player with `gamertag` on PC `i`'s friends list, as last sent.
fn listed(w: &World, i: usize, gamertag: &str) -> Option<Friend> {
    let list = &w.pcs[i].view.friends;
    list.iter().find(|f| f.gamertag == gamertag).cloned()
}

/// How many friends lists PC `i` has had.
fn lists(w: &World, i: usize) -> usize {
    let came = |e: &&LiveEvent| **e == LiveEvent::Friends;
    w.events[i].iter().filter(came).count()
}

fn gamertag(w: &World, i: usize) -> String {
    w.welcome(i).gamertag.clone()
}

/// PCs `a` and `b` (launchers) become friends: `a` asks, `b` accepts.
fn befriend(w: &mut World, a: usize, b: usize) {
    let (tag_a, tag_b) = (gamertag(w, a), gamertag(w, b));
    w.send(a, ToServer::FriendRequest(tag_b.clone()));
    w.until(|w| listed(w, b, &tag_a).is_some());
    w.send(b, ToServer::FriendAccept(w.id(a)));
    let friends = |w: &World, i: usize, tag: &str| {
        listed(w, i, tag).is_some_and(|f| f.relation == Relation::Friend)
    };
    w.until(|w| friends(w, a, &tag_b) && friends(w, b, &tag_a));
}

/// The service records PC `i` was sent, in order.
fn records(w: &World, i: usize) -> Vec<ServiceRecord> {
    let sent = w.events[i].iter().filter_map(|e| match e {
        LiveEvent::ServiceRecord(r) => Some(r.clone()),
        _ => None,
    });
    sent.collect()
}

#[test]
fn launchers_make_friends_by_gamertag() {
    let mut w = World::new("friends");
    let a = launcher(&mut w, 1, "Alpha");
    let b = launcher(&mut w, 2, "Bravo");
    // Each is sent a list on signing in: empty.
    w.until(|w| lists(w, a) > 0 && lists(w, b) > 0);
    assert!(w.pcs[a].view.friends.is_empty());
    // No one has that gamertag; we can't ask ourselves.
    w.send(a, ToServer::FriendRequest("Nobody".into()));
    w.until(|w| w.notices(a).len() == 1);
    assert_eq!(w.notices(a), [live::NO_SUCH_PLAYER]);
    w.send(a, ToServer::FriendRequest(" alpha".into()));
    w.until(|w| w.notices(a).len() == 2);
    assert_eq!(w.notices(a)[1], live::NOT_YOURSELF);
    // A gamertag in any case finds them; asking twice is told.
    w.send(a, ToServer::FriendRequest("bravo".into()));
    w.until(|w| w.notices(a).len() == 3 && !w.notices(b).is_empty());
    assert_eq!(w.notices(a)[2], "FRIEND REQUEST SENT TO BRAVO");
    assert_eq!(w.notices(b), ["ALPHA SENT YOU A FRIEND REQUEST"]);
    w.send(a, ToServer::FriendRequest("BRAVO".into()));
    w.until(|w| w.notices(a).len() == 4);
    assert_eq!(w.notices(a)[3], "YOU ALREADY SENT BRAVO A FRIEND REQUEST");
    // Bravo sees the request: the gamertag and level only.
    w.until(|w| listed(w, b, "ALPHA").is_some());
    let asked = listed(&w, b, "ALPHA").unwrap();
    let request = Friend {
        account: w.id(a),
        gamertag: "ALPHA".into(),
        relation: Relation::AskedUs,
        best: 1,
        online: Online::Offline,
        activity: Activity::Lobby,
        playlist: 0,
        map: String::new(),
        variant: String::new(),
        party: 0,
        joinable: false,
    };
    assert_eq!(asked, request);
    w.until(|w| listed(w, a, "BRAVO").is_some());
    assert_eq!(listed(&w, a, "BRAVO").unwrap().relation, Relation::WeAsked);
    // Accepted, both are friends, online on the launcher, in a lobby, and
    // can join each other's party.
    w.send(b, ToServer::FriendAccept(w.id(a)));
    let friend = |w: &World, i: usize, tag: &str| {
        listed(w, i, tag).filter(|f| f.relation == Relation::Friend)
    };
    w.until(|w| friend(w, a, "BRAVO").is_some() && friend(w, b, "ALPHA").is_some());
    for (i, tag, other) in [(a, "BRAVO", b), (b, "ALPHA", a)] {
        let f = friend(&w, i, tag).unwrap();
        assert_eq!((f.online, f.activity), (Online::Launcher, Activity::Lobby));
        assert_eq!(f.party, w.party(other).id);
        assert!(f.joinable);
    }
    assert_eq!(w.notices(a)[4], "BRAVO ACCEPTED YOUR FRIEND REQUEST");
    assert_eq!(w.notices(b).len(), 1);
    // Asking a friend is told.
    w.send(b, ToServer::FriendRequest("alpha".into()));
    w.until(|w| w.notices(b).len() == 2);
    assert_eq!(w.notices(b)[1], "ALPHA IS ALREADY YOUR FRIEND");
    // It's on disk.
    let (x, y) = (w.id(a).min(w.id(b)), w.id(a).max(w.id(b)));
    let text = std::fs::read_to_string(w.data.0.join("friends.txt")).unwrap();
    assert_eq!(text, format!("f {x:016x} {y:016x}\n"));
}

#[test]
fn requests_both_ways_make_friends_at_once() {
    let mut w = World::new("friends-crossed");
    let a = launcher(&mut w, 1, "Alpha");
    let b = launcher(&mut w, 2, "Bravo");
    w.send(a, ToServer::FriendRequest("BRAVO".into()));
    w.until(|w| !w.notices(b).is_empty());
    w.send(b, ToServer::FriendRequest("ALPHA".into()));
    w.until(|w| w.notices(b).len() == 2 && w.notices(a).len() == 2);
    assert_eq!(w.notices(a)[1], "YOU AND BRAVO ARE NOW FRIENDS");
    assert_eq!(w.notices(b)[1], "YOU AND ALPHA ARE NOW FRIENDS");
    w.until(|w| listed(w, a, "BRAVO").is_some_and(|f| f.relation == Relation::Friend));
    assert!(w.server.friends.are_friends(w.id(a), w.id(b)));
    assert_eq!(w.server.friends.count(w.id(a)), 1);
}

#[test]
fn friends_see_what_friends_are_doing() {
    let mut w = World::new("friends-doing");
    let relay = relay(&mut w);
    let a = launcher(&mut w, 1, "Alpha");
    let b = launcher(&mut w, 2, "Bravo");
    let f = launcher(&mut w, 6, "Foxtrot");
    befriend(&mut w, a, b);
    let alpha = |w: &World| listed(w, b, "ALPHA").unwrap();
    let party = w.party(a).id;
    assert_eq!((alpha(&w).party, alpha(&w).joinable), (party, true));

    // A custom game: playing, playlist 255, with its map and game.
    w.send(
        a,
        ToServer::LauncherCustom {
            map: "lockout".into(),
            variant: "H2_Team_Slayer".into(),
        },
    );
    w.until(|w| alpha(w).activity == Activity::Playing);
    let seen = alpha(&w);
    assert_eq!((seen.playlist, seen.joinable), (CUSTOM_GAME, false));
    assert_eq!(
        (seen.map.as_str(), seen.variant.as_str()),
        ("lockout", "H2_Team_Slayer")
    );
    // Joining on the flag anyway gets JOIN_PARTY's own answer.
    let notices = w.notices(b).len();
    w.send(b, ToServer::JoinParty(seen.party));
    w.until(|w| w.notices(b).len() > notices);
    assert_eq!(w.notices(b).last().unwrap(), "THAT PARTY IS IN A MATCH");
    let custom = launcher_matches(&w, a).pop().unwrap();
    w.send(
        a,
        ToServer::LeftMatch {
            id: custom.id,
            host_lost: false,
        },
    );
    w.until(|w| alpha(w).activity == Activity::Lobby);
    assert!(alpha(&w).map.is_empty() && alpha(&w).joinable);

    // Searching, then playing, Head to Head.
    w.send(a, ToServer::Search(HEAD_TO_HEAD));
    w.until(|w| alpha(w).activity == Activity::Searching);
    assert_eq!(
        (alpha(&w).playlist, alpha(&w).joinable),
        (HEAD_TO_HEAD, true)
    );
    w.send(f, ToServer::Search(HEAD_TO_HEAD));
    w.until(|w| alpha(w).activity == Activity::Playing);
    let seen = alpha(&w);
    let m = launcher_matches(&w, a).pop().unwrap();
    assert_eq!((seen.playlist, seen.joinable), (HEAD_TO_HEAD, false));
    assert_eq!((&seen.map, &seen.variant), (&m.map, &m.variant));
    assert!(MAPS.contains(&seen.map.as_str()));
    w.send(
        a,
        ToServer::LeftMatch {
            id: m.id,
            host_lost: false,
        },
    );
    w.until(|w| alpha(w).activity == Activity::Lobby);

    // Invite only, and Bravo not invited: not joinable, and JOIN_PARTY
    // says the same.
    w.send(a, ToServer::Privacy(Privacy::InviteOnly));
    w.until(|w| !alpha(w).joinable);
    let notices = w.notices(b).len();
    w.send(b, ToServer::JoinParty(party));
    w.until(|w| w.notices(b).len() > notices);
    assert_eq!(w.notices(b).last().unwrap(), "THAT PARTY IS INVITE ONLY");
    // Invited: the invite alone brings a new list, and it's joinable.
    let before = lists(&w, b);
    w.send(a, ToServer::Invite(w.id(b)));
    w.until(|w| alpha(w).joinable);
    assert!(lists(&w, b) > before);
    // Joined with the entry's party: our own party isn't joinable.
    w.send(b, ToServer::JoinParty(alpha(&w).party));
    w.until(|w| w.members(a).contains(&w.id(b)));
    w.until(|w| alpha(w).party == w.party(b).id);
    w.run(1.5);
    assert!(!alpha(&w).joinable);
    assert!(!listed(&w, a, "BRAVO").unwrap().joinable);

    // Removed by the leader: open again, but not to Bravo.
    w.send(a, ToServer::Kick(w.id(b)));
    w.until(|w| w.notices(b).last().is_some_and(|n| n.contains("REMOVED")));
    w.send(a, ToServer::Privacy(Privacy::Open));
    w.until(|w| w.party(a).privacy == Privacy::Open);
    w.run(1.5);
    assert_eq!(alpha(&w).party, party);
    assert!(!alpha(&w).joinable);
    let notices = w.notices(b).len();
    w.send(b, ToServer::JoinParty(party));
    w.until(|w| w.notices(b).len() > notices);
    assert_eq!(w.notices(b).last().unwrap(), "THAT PARTY IS INVITE ONLY");

    // Full, counting Bravo's guests: thirteen people in Alpha's party fit
    // Bravo alone, but not with three guests.
    w.send(a, ToServer::Invite(w.id(b)));
    w.until(|w| alpha(w).joinable);
    for (n, tag) in [(3, "Charlie"), (4, "Delta"), (5, "Echo")] {
        let i = launcher(&mut w, n, tag);
        w.send(i, ToServer::JoinParty(party));
        w.until(|w| w.members(a).contains(&w.id(i)));
        w.send(i, ToServer::Guests(3));
        w.until(|w| {
            w.party(i)
                .members
                .iter()
                .any(|m| m.guests == 3 && m.account == w.id(i))
        });
    }
    w.run(1.5);
    assert!(alpha(&w).joinable);
    w.send(b, ToServer::Guests(3));
    w.until(|w| !alpha(w).joinable);
    let notices = w.notices(b).len();
    w.send(b, ToServer::JoinParty(party));
    w.until(|w| w.notices(b).len() > notices);
    assert_eq!(w.notices(b).last().unwrap(), "THE PARTY IS FULL");
    // (Agreeing with JOIN_PARTY case by case, by the same checks.)
    assert_eq!(w.server.can_join(w.id(b), party), Err("THE PARTY IS FULL"));
    drop(relay);
}

#[test]
fn joinable_follows_invites_declined_and_pushed_out() {
    let mut w = World::new("friends-invites");
    let a = launcher(&mut w, 1, "Alpha");
    let b = launcher(&mut w, 2, "Bravo");
    befriend(&mut w, a, b);
    let alpha = |w: &World| listed(w, b, "ALPHA").unwrap();
    let party = w.party(a).id;
    w.send(a, ToServer::Privacy(Privacy::InviteOnly));
    w.until(|w| !alpha(w).joinable);
    // Invited, then declined: no longer joinable.
    w.send(a, ToServer::Invite(w.id(b)));
    w.until(|w| alpha(w).joinable);
    w.send(b, ToServer::Decline(party));
    w.until(|w| !alpha(w).joinable);
    // Invited, then pushed out by 32 newer invites (the cap): the same.
    w.send(a, ToServer::Invite(w.id(b)));
    w.until(|w| alpha(w).joinable);
    for n in 0..32 {
        let i = launcher(&mut w, 10 + n, &format!("Guest{n}"));
        w.send(a, ToServer::Invite(w.id(i)));
    }
    w.until(|w| !alpha(w).joinable);
    assert_eq!(
        w.server.can_join(w.id(b), party),
        Err("THAT PARTY IS INVITE ONLY")
    );

    // Removed from an open party, then pushed out of its 32 removed
    // players: joinable again.
    w.send(a, ToServer::Privacy(Privacy::Open));
    w.send(b, ToServer::JoinParty(party));
    w.until(|w| w.members(a).contains(&w.id(b)));
    w.send(a, ToServer::Kick(w.id(b)));
    w.until(|w| !w.members(a).contains(&w.id(b)));
    w.until(|w| alpha(w).party == party && !alpha(w).joinable);
    for i in 2..34 {
        w.send(i, ToServer::JoinParty(party));
        w.until(|w| w.members(a).contains(&w.id(i)));
        w.send(a, ToServer::Kick(w.id(i)));
        w.until(|w| !w.members(a).contains(&w.id(i)));
    }
    w.until(|w| alpha(w).joinable);
    assert_eq!(w.server.can_join(w.id(b), party), Ok(()));
}

#[test]
fn requests_show_new_gamertags_both_ways() {
    let mut w = World::new("friends-renamed");
    let a = launcher(&mut w, 1, "Alpha");
    let b = launcher(&mut w, 2, "Bravo");
    w.send(a, ToServer::FriendRequest("BRAVO".into()));
    w.until(|w| listed(w, b, "ALPHA").is_some() && listed(w, a, "BRAVO").is_some());
    // The one asked renames: the asker's "request sent" row follows.
    w.send(
        b,
        ToServer::Profile {
            gamertag: "Kat".into(),
            look: Look::default(),
        },
    );
    w.until(|w| listed(w, a, "KAT").is_some_and(|f| f.relation == Relation::WeAsked));
    // The asker renames: the "requests to you" row follows.
    w.send(
        a,
        ToServer::Profile {
            gamertag: "Jun".into(),
            look: Look::default(),
        },
    );
    w.until(|w| listed(w, b, "JUN").is_some_and(|f| f.relation == Relation::AskedUs));
}

#[test]
fn declining_and_removing_are_quiet() {
    let mut w = World::new("friends-quiet");
    let a = launcher(&mut w, 1, "Alpha");
    let b = launcher(&mut w, 2, "Bravo");
    let c = launcher(&mut w, 3, "Charlie");
    // A decline: the request goes from Alpha's list, unsaid.
    w.send(a, ToServer::FriendRequest("BRAVO".into()));
    w.until(|w| listed(w, b, "ALPHA").is_some() && listed(w, a, "BRAVO").is_some());
    w.send(b, ToServer::FriendDecline(w.id(a)));
    w.until(|w| listed(w, a, "BRAVO").is_none() && listed(w, b, "ALPHA").is_none());
    // A removal: Bravo's list empties, unsaid.
    befriend(&mut w, a, b);
    w.send(a, ToServer::FriendRemove(w.id(b)));
    w.until(|w| w.pcs[a].view.friends.is_empty() && w.pcs[b].view.friends.is_empty());
    w.run(1.5);
    let said = |w: &World, i: usize| w.notices(i).to_vec();
    assert_eq!(
        said(&w, a),
        [
            "FRIEND REQUEST SENT TO BRAVO",
            "FRIEND REQUEST SENT TO BRAVO",
            "BRAVO ACCEPTED YOUR FRIEND REQUEST"
        ]
    );
    assert_eq!(said(&w, b), ["ALPHA SENT YOU A FRIEND REQUEST"]);
    // Accepting a request taken back a moment before changes nothing and
    // says nothing, but the list comes again.
    w.send(c, ToServer::FriendRequest("BRAVO".into()));
    w.until(|w| listed(w, b, "CHARLIE").is_some());
    w.send(c, ToServer::FriendRemove(w.id(b)));
    w.until(|w| listed(w, b, "CHARLIE").is_none());
    w.run(1.5);
    let (before, list) = (lists(&w, b), w.pcs[b].view.friends.clone());
    w.send(b, ToServer::FriendAccept(w.id(c)));
    w.run(1.5);
    assert_eq!(lists(&w, b), before + 1);
    assert_eq!(w.pcs[b].view.friends, list);
    assert!(!w.server.friends.are_friends(w.id(b), w.id(c)));
    // (Only the request was heard of; nothing since.)
    assert_eq!(said(&w, b)[1..], ["CHARLIE SENT YOU A FRIEND REQUEST"]);
    assert_eq!(said(&w, c), ["FRIEND REQUEST SENT TO BRAVO"]);
    // The same for a decline or a removal of nothing.
    for message in [
        ToServer::FriendDecline(w.id(c)),
        ToServer::FriendRemove(w.id(c)),
    ] {
        let before = lists(&w, b);
        w.send(b, message);
        w.run(1.5);
        assert_eq!(lists(&w, b), before + 1);
    }
    assert_eq!(said(&w, b).len(), 2);
}

#[test]
fn friends_outlive_a_restart() {
    let mut w = World::new("friends-restart");
    let a = launcher(&mut w, 1, "Alpha");
    let b = launcher(&mut w, 2, "Bravo");
    let c = launcher(&mut w, 3, "Charlie");
    befriend(&mut w, a, b);
    w.send(c, ToServer::FriendRequest("ALPHA".into()));
    w.until(|w| listed(w, a, "CHARLIE").is_some());
    let (ids, text) = ([w.id(a), w.id(b), w.id(c)], w.server.friends.text());
    w.restart();
    assert_eq!(w.server.friends.text(), text);
    let a = launcher(&mut w, 1, "Alpha");
    w.until(|w| w.pcs[a].view.friends.len() == 2);
    // The request first, then the friend (offline now).
    let list = &w.pcs[a].view.friends;
    assert_eq!(
        (list[0].account, list[0].relation),
        (ids[2], Relation::AskedUs)
    );
    assert_eq!(
        (list[1].account, list[1].relation),
        (ids[1], Relation::Friend)
    );
    assert_eq!(list[1].online, Online::Offline);
    // Bravo signs in: Alpha sees them online.
    launcher(&mut w, 2, "Bravo");
    w.until(|w| listed(w, a, "BRAVO").is_some_and(|f| f.online == Online::Launcher));
}

#[test]
fn todays_data_folder_opens_as_it_is() {
    let mut w = World::new("friends-old-data");
    // accounts.txt as the server before friends and tallies wrote it, and
    // no friends.txt.
    let key = key(1).verifying_key().to_bytes();
    let id = store::account_id(&key);
    let mut look = Writer::default();
    Look::default().write(&mut look);
    let accounts = format!(
        "a {id:016x} {} OLD TIMER {} 1700000000 4\n\
         x {id:016x} mcc_head_to_head 120 2 3 2\n",
        store::hex(&key),
        store::hex(&look.0)
    );
    std::fs::write(w.data.0.join("accounts.txt"), &accounts).unwrap();
    w.restart();
    assert!(!w.data.0.join("friends.txt").exists());
    let a = launcher(&mut w, 1, "Old Timer");
    assert_eq!(w.welcome(a).gamertag, "OLD TIMER");
    assert_eq!(w.welcome(a).levels, [(HEAD_TO_HEAD, 2, 3)]);
    w.send(a, ToServer::Record(id));
    w.until(|w| !records(w, a).is_empty());
    let record = records(&w, a).remove(0).found.unwrap();
    assert_eq!(record.playlists.len(), 1);
    let p = record.playlists[0];
    assert_eq!(
        (p.playlist, p.level, p.games, p.wins),
        (HEAD_TO_HEAD, 2, 3, 2)
    );
    assert_eq!(p.tally, h2net::live::Tally::default());
    assert_eq!(record.created, 1_700_000_000);
    // Its account is written back as it was (but for its new seq).
    let x = w.accounts_txt();
    assert!(
        x.ends_with(&format!("x {id:016x} mcc_head_to_head 120 2 3 2\n")),
        "{x}"
    );
}

#[test]
fn friend_lists_come_at_most_once_a_second() {
    let mut w = World::new("friends-flood");
    let a = launcher(&mut w, 1, "Alpha");
    let b = launcher(&mut w, 2, "Bravo");
    befriend(&mut w, a, b);
    w.run(1.5);
    // Ten changes within a second.
    let before = lists(&w, a);
    for i in 0..10 {
        let privacy = [Privacy::InviteOnly, Privacy::Open][i % 2];
        w.send(b, ToServer::Privacy(privacy));
        w.step();
        w.step();
    }
    assert!(
        lists(&w, a) - before <= 2,
        "{} lists",
        lists(&w, a) - before
    );
    // The last list is right: Bravo's party is open again.
    w.run(1.5);
    assert_eq!(w.party(b).privacy, Privacy::Open);
    assert!(listed(&w, a, "BRAVO").unwrap().joinable);
    // Nothing changing, nothing is sent.
    let before = lists(&w, a);
    w.run(5.0);
    assert_eq!(lists(&w, a), before);
}

#[test]
fn friend_limits_outlast_signing_in_again() {
    let mut w = World::new("friends-limits");
    let a = launcher(&mut w, 1, "Alpha");
    let b = launcher(&mut w, 2, "Bravo");
    // 30 friend actions in a minute (each finding nothing to change)...
    for _ in 0..30 {
        w.send(a, ToServer::FriendRemove(w.id(b)));
    }
    w.step();
    // ... and the 31st and 32nd do nothing, with one notice.
    w.send(a, ToServer::FriendRequest("BRAVO".into()));
    w.send(a, ToServer::FriendRequest("BRAVO".into()));
    w.run(1.5);
    assert_eq!(w.notices(a), [live::SLOW_DOWN]);
    assert!(w.pcs[b].view.friends.is_empty());
    // Signing in again doesn't start the count afresh.
    let a = launcher(&mut w, 1, "Alpha");
    w.send(a, ToServer::FriendRequest("BRAVO".into()));
    w.run(1.5);
    assert!(w.notices(a).is_empty());
    assert!(w.pcs[b].view.friends.is_empty());
    // A minute on, it works.
    w.run(60.0);
    w.send(a, ToServer::FriendRequest("BRAVO".into()));
    w.until(|w| listed(w, b, "ALPHA").is_some());
    assert_eq!(w.notices(a), ["FRIEND REQUEST SENT TO BRAVO"]);

    // 20 RECORDs answered in 10 seconds, signing in again or not.
    for _ in 0..25 {
        w.send(a, ToServer::Record(w.id(b)));
    }
    w.run(1.0);
    let a = launcher(&mut w, 1, "Alpha");
    for _ in 0..25 {
        w.send(a, ToServer::Record(w.id(b)));
    }
    w.run(1.0);
    let answered: usize = (0..w.pcs.len()).map(|i| records(&w, i).len()).sum();
    assert_eq!(answered, 20);
    w.run(10.0);
    w.send(a, ToServer::Record(w.id(b)));
    w.run(0.5);
    assert_eq!(records(&w, a).len(), 1);

    // Asking, taking it back and asking again: the one asked hears once.
    let c = launcher(&mut w, 3, "Charlie");
    let d = launcher(&mut w, 4, "Delta");
    for _ in 0..3 {
        w.send(c, ToServer::FriendRequest("DELTA".into()));
        w.step();
        w.send(c, ToServer::FriendRemove(w.id(d)));
        w.step();
    }
    w.send(c, ToServer::FriendRequest("DELTA".into()));
    w.until(|w| listed(w, d, "CHARLIE").is_some());
    w.run(1.5);
    assert_eq!(w.notices(d), ["CHARLIE SENT YOU A FRIEND REQUEST"]);
    assert_eq!(w.notices(c).len(), 4);
}

#[test]
fn the_game_never_hears_of_friends() {
    let mut w = World::new("friends-game");
    let mut game = sign_in_by_hand(&mut w, 1, "Gamer");
    let a = launcher(&mut w, 2, "Alpha");
    // A launcher can ask a game player's account; the game hears nothing.
    w.send(a, ToServer::FriendRequest("gamer".into()));
    w.until(|w| !w.notices(a).is_empty());
    assert_eq!(w.notices(a), ["FRIEND REQUEST SENT TO GAMER"]);
    w.until(|w| listed(w, a, "GAMER").is_some());
    assert_eq!(listed(&w, a, "GAMER").unwrap().relation, Relation::WeAsked);
    w.send(a, ToServer::Record(w.id(a)));
    w.run(3.0);
    let messages = game.receive().unwrap();
    assert!(!messages.is_empty());
    for (kind, body) in messages {
        assert!(
            ![kind::SERVICE_RECORD, kind::FRIENDS].contains(&kind),
            "kind {kind}"
        );
        if let Ok(ToPc::Notice(text)) = ToPc::read(kind, &body) {
            assert!(!is_friend_notice(&text), "{text}");
        }
    }
    // The game sending a friend request, or a RECORD, is dropped.
    let online = w.server.players_online();
    ToServer::FriendRequest("ALPHA".into()).send(&mut game);
    game.flush().unwrap();
    w.step();
    assert_eq!(w.server.players_online(), online - 1);
    assert!(listed(&w, a, "GAMER").is_some());
    let mut game = sign_in_by_hand(&mut w, 1, "Gamer");
    ToServer::Record(w.id(a)).send(&mut game);
    game.flush().unwrap();
    w.step();
    assert_eq!(w.server.players_online(), online - 1);
    assert!(w.server.friends.asking(w.id(a)).len() == 1);
}

/// Every PC in `pcs` (launchers in one party each, all searching) is told
/// of a new match, its host hosts it, the others join it, and it starts:
/// the match.
fn play_launcher_match(w: &mut World, pcs: &[usize]) -> LauncherMatch {
    let told: Vec<usize> = pcs.iter().map(|&i| launcher_matches(w, i).len()).collect();
    w.until(|w| {
        pcs.iter()
            .zip(&told)
            .all(|(&i, &n)| launcher_matches(w, i).len() > n)
    });
    let m = launcher_matches(w, pcs[0]).pop().unwrap();
    for &i in pcs {
        if super::super::matches::relay_id(w.id(i)) == m.host {
            w.send(i, ToServer::Hosting(m.id));
        } else {
            w.send(i, ToServer::Joined(m.id));
        }
    }
    w.until(|w| {
        pcs.iter()
            .all(|&i| w.events[i].contains(&LiveEvent::Go(m.id)))
    });
    m
}

/// How match `id` ended for PC `i`, once it has.
fn match_over(w: &World, i: usize, id: u64) -> Option<MatchOver> {
    w.events[i].iter().find_map(|e| match e {
        LiveEvent::MatchOver(o) if o.id == id => Some(o.clone()),
        _ => None,
    })
}

#[test]
fn service_records_and_tallies() {
    let mut w = World::new("records");
    let relay = relay(&mut w);
    let a = launcher(&mut w, 1, "Alpha");
    let b = launcher(&mut w, 2, "Bravo");
    w.send(a, ToServer::Search(HEAD_TO_HEAD));
    w.send(b, ToServer::Search(HEAD_TO_HEAD));
    let m = play_launcher_match(&mut w, &[a, b]);
    assert_eq!((m.playlist, m.ranked), (HEAD_TO_HEAD, true));
    let relay_id = super::super::matches::relay_id;
    let (ra, rb) = (relay_id(w.id(a)), relay_id(w.id(b)));
    let host = if ra == m.host { a } else { b };
    // Alpha wins. The host's counts are the ones kept; the other PC's
    // differ, but agree on the places, so the game counts.
    let result = |kills_a: u16| LauncherResult {
        id: m.id,
        finished: true,
        team_scores: Vec::new(),
        players: vec![
            LauncherPlayerResult {
                relay_id: ra,
                team: 0,
                place: 0,
                score: 15,
                kills: kills_a,
                assists: 2,
                deaths: 5,
                betrayals: 0,
                suicides: 1,
                left: false,
            },
            LauncherPlayerResult {
                relay_id: rb,
                team: 0,
                place: 1,
                score: 5,
                kills: 6,
                assists: 1,
                deaths: 15,
                betrayals: 1,
                suicides: 0,
                left: false,
            },
        ],
    };
    w.send(host, ToServer::LauncherResult(result(16)));
    w.send(a + b - host, ToServer::LauncherResult(result(17)));
    w.until(|w| match_over(w, a, m.id).is_some() && match_over(w, b, m.id).is_some());
    assert!(match_over(&w, a, m.id).unwrap().counted);
    // Alpha's record, as Bravo sees it: one game, won, the host's numbers.
    w.send(b, ToServer::Record(w.id(a)));
    w.until(|w| !records(w, b).is_empty());
    let record = records(&w, b).remove(0);
    assert_eq!(record.account, w.id(a));
    let found = record.found.unwrap();
    assert_eq!(found.gamertag, "ALPHA");
    let [p] = found.playlists[..] else {
        panic!("{:?}", found.playlists);
    };
    assert_eq!((p.playlist, p.games, p.wins), (HEAD_TO_HEAD, 1, 1));
    let alphas = h2net::live::Tally {
        kills: 16,
        assists: 2,
        deaths: 5,
        betrayals: 0,
        suicides: 1,
    };
    assert_eq!(p.tally, alphas);
    assert_eq!(found.best, w.server.account(w.id(a)).unwrap().best_level());
    // On disk: 11-word playlist lines, and 14-field games.log entries.
    let accounts = w.accounts_txt();
    let lines: Vec<&str> = accounts
        .lines()
        .filter(|l| l.contains(" mcc_head_to_head "))
        .collect();
    assert_eq!(lines.len(), 2);
    assert!(
        lines.iter().all(|l| l.split(' ').count() == 12),
        "{lines:?}"
    );
    assert!(accounts.contains(" 1 1 16 2 5 0 1\n"), "{accounts}");
    assert!(accounts.contains(" 1 0 6 1 15 1 0\n"), "{accounts}");
    let log = std::fs::read_to_string(w.data.0.join("games.log")).unwrap();
    let entry_a = format!("{:016x}:", w.id(a));
    let entries: Vec<&str> = log.split(' ').filter(|e| e.contains(':')).collect();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|e| e.split(':').count() == 14));
    let alpha = entries.iter().find(|e| e.starts_with(&entry_a)).unwrap();
    assert!(alpha.ends_with(":15:16:2:5:0:1"), "{alpha}");
    // No such account; and at most 20 answers in 10 seconds.
    w.send(b, ToServer::Record(12345));
    w.until(|w| records(w, b).len() == 2);
    assert_eq!(records(&w, b)[1].found, None);
    w.run(11.0);
    for _ in 0..25 {
        w.send(b, ToServer::Record(w.id(b)));
    }
    w.run(1.0);
    assert_eq!(records(&w, b).len(), 2 + 20);

    // A disputed game adds nothing.
    w.until(|w| w.party(a).activity == Activity::Lobby && w.party(b).activity == Activity::Lobby);
    w.send(a, ToServer::Search(HEAD_TO_HEAD));
    w.send(b, ToServer::Search(HEAD_TO_HEAD));
    let m = play_launcher_match(&mut w, &[a, b]);
    let mut other = result(16);
    other.id = m.id;
    let mut theirs = other.clone();
    for p in &mut theirs.players {
        p.place = 1 - p.place;
    }
    let host = if ra == m.host { a } else { b };
    w.send(host, ToServer::LauncherResult(other));
    w.send(a + b - host, ToServer::LauncherResult(theirs));
    w.until(|w| match_over(w, a, m.id).is_some());
    let over = match_over(&w, a, m.id).unwrap();
    assert!(!over.counted);
    assert_eq!(
        over.reason,
        "THE RESULTS DIDN'T AGREE. THE GAME DIDN'T COUNT."
    );
    let stats = w.server.account(w.id(a)).unwrap().stats("mcc_head_to_head");
    let stats = stats.unwrap();
    assert_eq!(
        (stats.games, stats.tally),
        (1, crate::store::Tally::from(alphas))
    );
    drop(relay);
}
