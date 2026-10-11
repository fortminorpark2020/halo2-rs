//! Accounts the server's owner makes: signing in with a password once per
//! PC, the limits on wrong passwords, admins changing accounts from their
//! launcher and from the command line, sign-up, and that older folders and
//! the game go on as they were.

use super::*;
use crate::admin::{self, Request};
use crate::client::{Access, AccountAsk, SERVER_UNPROVEN};
use crate::password::{self, Cost, Secret};
use crate::server::accounts::{
    AN_ADMINS, LAST_ADMIN, NOT_A_HASH, NO_PASSWORD_YET, NO_SUCH_ACCOUNT, ONLY_THE_COMMAND_LINE,
    PASSWORD_CHANGED, RENAMES, WRONG_OLD_PASSWORD, YOUR_OWN,
};
use ed25519_dalek::{Signature, VerifyingKey};
use h2net::live::{AccountOp, Accounts, Relation, Sealed, ROW_ADMIN, ROW_ONLINE, ROW_PASSWORD};

const PASSWORD: &str = "K7MP-4QR9-XW2T";
const OTHER_PASSWORD: &str = "ANOTHER-PASSWORD";

/// A server whose owner makes the accounts, hashing cheaply.
fn closed(name: &str) -> World {
    let mut w = World::with_secret(name, Some(SECRET));
    w.server.set_sign_up(SignUp::Closed);
    w.server.set_password_cost(Cost::Cheap);
    w
}

/// Start the server again from its folder, closed as before.
fn restart_closed(w: &mut World) {
    w.restart();
    w.server.set_sign_up(SignUp::Closed);
    w.server.set_password_cost(Cost::Cheap);
}

/// An account made from the command line, with `password`. Its id.
fn make_account(w: &mut World, gamertag: &str, password: &str, admin: bool) -> u64 {
    let phc = password::hash(password, Cost::Cheap).unwrap();
    let add = Command::Add {
        gamertag: gamertag.into(),
        phc,
        admin,
    };
    w.server.command(Actor::Console, add, w.now).unwrap();
    w.server.find(gamertag).unwrap()
}

/// PC number `n` starts signing in as a launcher, as `gamertag`, with
/// `password` if any. Its index in `pcs`.
fn connect_launcher(w: &mut World, n: u8, gamertag: &str, password: Option<&str>) -> usize {
    connect_launcher_from(w, n, ip(n), gamertag, password)
}

/// `connect_launcher`, from the address `from`.
fn connect_launcher_from(
    w: &mut World,
    n: u8,
    from: IpAddr,
    gamertag: &str,
    password: Option<&str>,
) -> usize {
    let (server_end, pc_end) = Connection::pair();
    w.server.accept(server_end, Route::Live, from, w.now);
    let card = w.card_path(n);
    let mut pc = LiveClient::launcher(pc_end, key(n), &profile(gamertag), "test", &card, w.now);
    pc.set_access(Access {
        pin: None,
        password: password.map(Secret::new),
        computer: format!("PC {n}"),
    });
    w.pcs.push(pc);
    w.events.push(Vec::new());
    w.pcs.len() - 1
}

/// PC number `n` signs in as a launcher, as `connect_launcher` starts to.
fn launcher_in(w: &mut World, n: u8, gamertag: &str, password: Option<&str>) -> usize {
    let i = connect_launcher(w, n, gamertag, password);
    w.until(|w| w.pcs[i].signed_in() && w.pcs[i].view.party.is_some());
    i
}

/// Why PC `i` was turned away, once it is.
fn refusal(w: &mut World, i: usize) -> String {
    w.until(|w| w.refused(i).is_some());
    w.refused(i).unwrap().to_string()
}

/// PC number `n` tries to sign in as a launcher, and is turned away: why.
fn launcher_refused(w: &mut World, n: u8, gamertag: &str, password: Option<&str>) -> String {
    let i = connect_launcher(w, n, gamertag, password);
    refusal(w, i)
}

/// PC `i`'s accounts, as last sent.
fn accounts(w: &World, i: usize) -> &Accounts {
    w.pcs[i].view.accounts.as_ref().unwrap()
}

/// PC `i` asks the server to change the accounts, and is answered.
fn ask(w: &mut World, i: usize, asked: AccountAsk) -> Accounts {
    let seq = w.pcs[i].ask_account(asked).unwrap();
    let answered = |w: &World| w.pcs[i].view.accounts.as_ref().map(|a| a.answers) == Some(seq);
    w.until(answered);
    accounts(w, i).clone()
}

/// Run until `account` isn't signed in anywhere.
fn signed_out(w: &mut World, account: u64) {
    w.until(|w| w.server.pc_of(account).is_none());
}

fn secret(text: &str) -> Secret {
    Secret::new(text)
}

/// Start signing PC `n` in by hand as a launcher: the connection, the
/// challenge, and the key to seal passwords to (which the server proved
/// is its own).
fn launcher_by_hand(w: &mut World, n: u8, gamertag: &str) -> (Connection, [u8; 32], [u8; 32]) {
    let mut conn = w.raw(n);
    let login = Login {
        client: LoginClient::Launcher {
            version: "by hand".into(),
        },
        key: key(n).verifying_key().to_bytes(),
        gamertag: gamertag.into(),
        look: Look::default(),
        card: String::new(),
        maps: Vec::new(),
        guests: 0,
    };
    ToServer::Login(login).send(&mut conn);
    conn.flush().unwrap();
    w.step();
    let [ToPc::SealKey {
        key: seal_key,
        server,
        sig,
    }, ToPc::Challenge { nonce, .. }] = heard(&mut conn)[..]
    else {
        panic!("no seal key and challenge");
    };
    assert_eq!(server, w.server.identity_key());
    let proof = seal::key_proof(&nonce, &seal_key);
    let identity = VerifyingKey::from_bytes(&server).unwrap();
    identity
        .verify_strict(&proof, &Signature::from_bytes(&sig))
        .unwrap();
    (conn, nonce, seal_key)
}

/// PC `n`'s signature over the challenge `nonce`.
fn signature(n: u8, nonce: &[u8; 32]) -> [u8; 64] {
    let proof = live::proof(nonce, key(n).verifying_key().as_bytes());
    key(n).sign(&proof).to_bytes()
}

/// `password`, sealed by PC `n` to sign in to `gamertag` on the connection
/// whose challenge was `nonce` and key `seal_key`.
fn sealed_sign_in(n: u8, nonce: &[u8; 32], seal_key: &[u8; 32], gamertag: &str) -> Sealed {
    let me = key(n).verifying_key().to_bytes();
    let aad = seal::aad(seal::SIGN_IN, nonce, &me, gamertag.as_bytes());
    seal::seal(seal_key, &aad, &seal::pad(&[PASSWORD]).unwrap()).unwrap()
}

#[test]
fn closed_sign_up_asks_a_new_launcher_for_its_password() {
    let mut w = closed("closed-launcher");
    assert!(w.server.sign_up_closed());
    // No account, no password: it's asked for one.
    let why = launcher_refused(&mut w, 1, "Kat", None);
    assert_eq!(why, live::PASSWORD_NEEDED);
    assert_eq!(w.accounts_txt(), "");
    // An account's gamertag without its password: the same.
    let kat = make_account(&mut w, "Kat", PASSWORD, false);
    let why = launcher_refused(&mut w, 2, "Kat", None);
    assert_eq!(why, live::PASSWORD_NEEDED);
    assert_eq!(w.server.pc_of(kat), None);
    assert!(w.server.devices_of(kat).is_empty());
}

#[test]
fn closed_sign_up_turns_new_game_pcs_away_with_readable_text() {
    let mut w = closed("closed-game");
    make_account(&mut w, "Kat", PASSWORD, false);
    let a = w.connect(1, "Gamer");
    assert_eq!(refusal(&mut w, a), live::NO_ACCOUNT);
    // Not even under an account's gamertag (the game has no password).
    let b = w.connect(2, "Kat");
    assert_eq!(refusal(&mut w, b), live::NO_ACCOUNT);
    assert_eq!(w.server.account_rows().len(), 1);
}

#[test]
fn accounts_from_an_older_server_keep_signing_in_from_their_pcs() {
    let mut w = World::with_secret("closed-veteran", Some(SECRET));
    let veteran = veteran(&mut w);
    w.server.set_sign_up(SignUp::Closed);
    // The game, from its PC, under any gamertag.
    let a = w.sign_in(1, "Someone Else");
    assert_eq!(w.welcome(a).account, veteran.id);
    assert_eq!(w.welcome(a).gamertag, "VETERAN");
    assert_eq!(w.welcome(a).levels, [(DOUBLE_TEAM, 13, 40)]);
    // And a launcher on that PC, with no password.
    let b = launcher_in(&mut w, 1, "Veteran", None);
    assert_eq!(w.welcome(b).account, veteran.id);
    let devices = w.server.devices_of(veteran.id);
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].key, key(1).verifying_key().to_bytes());
    assert_eq!(devices[0].name, crate::logins::OLD_PC);
    // So after a restart.
    restart_closed(&mut w);
    let a = w.sign_in(1, "Veteran");
    assert_eq!(w.welcome(a).account, veteran.id);
    assert_eq!(w.welcome(a).best, 13);
}

#[test]
fn a_right_password_remembers_the_pc_and_the_next_sign_in_needs_none() {
    let mut w = closed("right-password");
    let kat = make_account(&mut w, "Kat", PASSWORD, false);
    // Typed as it's read out, in any case.
    let i = launcher_in(&mut w, 1, "kat", Some("k7mp 4qr9-xw2t"));
    assert_eq!(w.welcome(i).account, kat);
    assert_eq!(w.welcome(i).gamertag, "KAT");
    let devices = w.server.devices_of(kat);
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].key, key(1).verifying_key().to_bytes());
    assert_eq!(devices[0].name, "PC 1");
    // That PC needs no password from now on, whatever gamertag it says.
    let i = launcher_in(&mut w, 1, "Someone", None);
    assert_eq!(w.welcome(i).account, kat);
    assert_eq!(w.welcome(i).gamertag, "KAT");
    restart_closed(&mut w);
    let i = launcher_in(&mut w, 1, "Kat", None);
    assert_eq!(w.welcome(i).account, kat);
    // Nothing secret is on its stat card or in accounts.txt.
    assert!(!w.card(1).contains("argon2"));
    assert!(!w.accounts_txt().contains("argon2"));
}

#[test]
fn a_second_pc_is_added_and_the_sixth_is_refused() {
    let mut w = closed("six-pcs");
    let kat = make_account(&mut w, "Kat", PASSWORD, false);
    for n in 1..=5 {
        let i = launcher_in(&mut w, n, "Kat", Some(PASSWORD));
        assert_eq!(w.welcome(i).account, kat);
        assert_eq!(w.server.devices_of(kat).len(), usize::from(n));
    }
    let why = launcher_refused(&mut w, 6, "Kat", Some(PASSWORD));
    assert_eq!(why, live::TOO_MANY_PCS);
    assert_eq!(w.server.devices_of(kat).len(), 5);
    // The five still sign in.
    let i = launcher_in(&mut w, 1, "Kat", None);
    assert_eq!(w.welcome(i).account, kat);
    // Signed out of them all, the sixth gets in.
    w.server
        .command(Actor::Console, Command::SignOut { account: kat }, w.now)
        .unwrap();
    let i = launcher_in(&mut w, 6, "Kat", Some(PASSWORD));
    assert_eq!(w.welcome(i).account, kat);
}

#[test]
fn a_password_sign_in_moves_this_pc_from_another_account() {
    let mut w = closed("move-pc");
    let kat = make_account(&mut w, "Kat", PASSWORD, false);
    let jun = make_account(&mut w, "Jun", OTHER_PASSWORD, false);
    let a = launcher_in(&mut w, 1, "Kat", Some(PASSWORD));
    assert_eq!(w.welcome(a).account, kat);
    // The same PC signs in to another account with its password: Kat's
    // session on it ends, and the PC is Jun's now.
    let b = launcher_in(&mut w, 1, "Jun", Some(OTHER_PASSWORD));
    assert_eq!(w.welcome(b).account, jun);
    assert_eq!(refusal(&mut w, a), live::SIGNED_OUT);
    assert!(w.server.devices_of(kat).is_empty());
    assert_eq!(w.server.devices_of(jun).len(), 1);
    let c = launcher_in(&mut w, 1, "Kat", None);
    assert_eq!(w.welcome(c).account, jun);
}

#[test]
fn a_wrong_password_and_an_unknown_gamertag_get_the_same_answer() {
    let mut w = closed("wrong-password");
    let kat = make_account(&mut w, "Kat", PASSWORD, false);
    let a = connect_launcher(&mut w, 1, "Kat", Some(OTHER_PASSWORD));
    let b = connect_launcher(&mut w, 2, "Nobody", Some(PASSWORD));
    w.until(|w| w.refused(a).is_some() && w.refused(b).is_some());
    assert_eq!(w.refused(a), Some(live::WRONG_PASSWORD));
    assert_eq!(w.events[a], w.events[b]);
    assert!(w.server.devices_of(kat).is_empty());
    // A password too long to be anyone's isn't even sent.
    let long = "A".repeat(65);
    let c = connect_launcher(&mut w, 3, "Kat", Some(&long));
    assert_eq!(refusal(&mut w, c), live::WRONG_PASSWORD);
}

#[test]
fn five_wrong_passwords_lock_new_pcs_out_but_remembered_pcs_still_sign_in() {
    let mut w = closed("locked");
    let kat = make_account(&mut w, "Kat", PASSWORD, false);
    launcher_in(&mut w, 1, "Kat", Some(PASSWORD));
    for n in 2..=6 {
        let why = launcher_refused(&mut w, n, "Kat", Some(OTHER_PASSWORD));
        assert_eq!(why, live::WRONG_PASSWORD);
    }
    // Locked: even the right password, from a new PC.
    let why = launcher_refused(&mut w, 7, "Kat", Some(PASSWORD));
    assert_eq!(why, live::TOO_MANY_TRIES);
    // The PC it knows needs none.
    let i = launcher_in(&mut w, 1, "Kat", None);
    assert_eq!(w.welcome(i).account, kat);
    // Ten minutes later, it's open again.
    w.run(600.0);
    let i = launcher_in(&mut w, 7, "Kat", Some(PASSWORD));
    assert_eq!(w.welcome(i).account, kat);
}

#[test]
fn an_ip_with_ten_wrong_passwords_is_locked() {
    let mut w = closed("ip-locked");
    let kat = make_account(&mut w, "Kat", PASSWORD, false);
    let from = ip(200);
    // Ten PCs at one address, each with a gamertag no one has (no more
    // than ten sign-ins a minute from it).
    for n in 1..=10 {
        if n == 6 {
            w.run(61.0);
        }
        let i = connect_launcher_from(&mut w, n, from, &format!("Nobody {n}"), Some(PASSWORD));
        assert_eq!(refusal(&mut w, i), live::WRONG_PASSWORD);
    }
    w.run(61.0);
    let i = connect_launcher_from(&mut w, 11, from, "Kat", Some(PASSWORD));
    assert_eq!(refusal(&mut w, i), live::TOO_MANY_TRIES);
    // From elsewhere, the account is no one's to lock.
    let i = launcher_in(&mut w, 12, "Kat", Some(PASSWORD));
    assert_eq!(w.welcome(i).account, kat);
}

#[test]
fn more_than_30_hashes_a_minute_make_the_server_busy() {
    let mut w = closed("busy");
    make_account(&mut w, "Kat", PASSWORD, true);
    let jun = make_account(&mut w, "Jun", OTHER_PASSWORD, false);
    let admin = launcher_in(&mut w, 1, "Kat", Some(PASSWORD));
    // That sign-in's hash is old news a minute later.
    w.run(61.0);
    for n in 0..30 {
        let password = secret(&format!("PASSWORD-{n:02}"));
        let set = AccountAsk::SetPassword {
            account: jun,
            password,
        };
        let answer = ask(&mut w, admin, set);
        assert!(answer.ok, "{n}: {}", answer.text);
    }
    let set = AccountAsk::SetPassword {
        account: jun,
        password: secret("ONE-TOO-MANY"),
    };
    let answer = ask(&mut w, admin, set);
    assert_eq!(
        (answer.ok, answer.text.as_str()),
        (false, live::SERVER_BUSY)
    );
    // A sign-in with a password waits too.
    let why = launcher_refused(&mut w, 2, "Jun", Some("PASSWORD-29"));
    assert_eq!(why, live::SERVER_BUSY);
    w.run(61.0);
    let i = launcher_in(&mut w, 2, "Jun", Some("PASSWORD-29"));
    assert_eq!(w.welcome(i).account, jun);
}

#[test]
fn a_box_from_another_connection_does_not_open() {
    let mut w = closed("other-box");
    let kat = make_account(&mut w, "Kat", PASSWORD, false);
    // Sealed for one connection...
    let (_first, nonce, seal_key) = launcher_by_hand(&mut w, 1, "KAT");
    let password = sealed_sign_in(1, &nonce, &seal_key, "KAT");
    // ...and sent on another, by the same PC.
    let (mut second, nonce, _) = launcher_by_hand(&mut w, 1, "KAT");
    let prove = ToServer::ProvePassword {
        signature: signature(1, &nonce),
        computer: "BY HAND".into(),
        password,
    };
    prove.send(&mut second);
    second.flush().unwrap();
    w.step();
    assert_eq!(
        heard(&mut second),
        [ToPc::Refused(live::WRONG_PASSWORD.into())]
    );
    assert!(w.server.devices_of(kat).is_empty());
    // Sealed for its own connection, it opens.
    let (mut third, nonce, seal_key) = launcher_by_hand(&mut w, 1, "KAT");
    let prove = ToServer::ProvePassword {
        signature: signature(1, &nonce),
        computer: "BY HAND".into(),
        password: sealed_sign_in(1, &nonce, &seal_key, "KAT"),
    };
    prove.send(&mut third);
    third.flush().unwrap();
    w.step();
    assert!(matches!(heard(&mut third)[0], ToPc::Welcome(_)));
    assert_eq!(w.server.devices_of(kat)[0].name, "BY HAND");
}

#[test]
fn prove_password_from_the_game_drops_the_pc() {
    let mut w = World::new("game-password");
    let (mut conn, nonce) = log_in_by_hand(&mut w, 1, "Gamer");
    let prove = ToServer::ProvePassword {
        signature: signature(1, &nonce),
        computer: "GAME".into(),
        password: Sealed {
            key: [9; 32],
            boxed: vec![0; live::SEALED_ONE + 16],
        },
    };
    prove.send(&mut conn);
    conn.flush().unwrap();
    w.step();
    assert_eq!(heard(&mut conn), []);
    assert_eq!(w.server.players_online(), 0);
    assert_eq!(w.accounts_txt(), "");
    w.run(3.0);
    assert!(conn.receive().is_err());
}

#[test]
fn the_game_never_hears_of_accounts() {
    let mut w = World::new("accounts-game");
    w.server.set_password_cost(Cost::Cheap);
    let mut game = sign_in_by_hand(&mut w, 1, "Gamer");
    let a = sign_in_launcher(&mut w, 2, "Alpha", &[]);
    // The launcher was sent the key to seal to, and its accounts.
    assert_eq!(w.pcs[a].view.server_key, Some(w.server.identity_key()));
    assert!(w.pcs[a].view.accounts.is_some());
    let came = |w: &World| {
        w.events[a]
            .iter()
            .filter(|e| **e == LiveEvent::Accounts)
            .count()
    };
    let before = came(&w);
    // An account changes: every launcher hears, the game doesn't.
    make_account(&mut w, "Kat", PASSWORD, false);
    w.run(3.0);
    assert_eq!(came(&w), before + 1);
    let messages = game.receive().unwrap();
    assert!(!messages.is_empty());
    for (kind, _) in messages {
        assert!(
            ![kind::SEAL_KEY, kind::ACCOUNTS].contains(&kind),
            "kind {kind}"
        );
    }
}

#[test]
fn a_launcher_at_version_3_is_told_to_update() {
    let mut w = World::new("live-v3");
    let mut conn = w.raw(1);
    // A LOGIN byte for byte as launchers at LIVE_PROTOCOL 3 wrote it.
    let mut old = Writer::default();
    old.u32(live::MAGIC_V2);
    old.u32(3);
    old.u8(live::CLIENT_LAUNCHER);
    old.str("h2launch 0.2.0");
    old.0.extend_from_slice(&key(1).verifying_key().to_bytes());
    old.str("ALPHA");
    Look::default().write(&mut old);
    old.str("");
    old.u16(1);
    old.str("lockout");
    old.u64(7);
    old.u8(0);
    conn.send(kind::LOGIN, &old.0);
    conn.flush().unwrap();
    w.step();
    assert_eq!(
        heard(&mut conn),
        [ToPc::Refused(live::UPDATE_YOUR_LAUNCHER.into())]
    );
    assert_eq!(w.server.players_online(), 0);
    assert_eq!(w.server.connections(), 0);
}

#[test]
fn account_requests_from_a_non_admin_or_badly_signed_or_replayed_drop_the_pc() {
    let mut w = World::with_secret("bad-requests", Some(SECRET));
    w.server.set_password_cost(Cost::Cheap);
    // A game player from before there was an admin.
    let mut game = sign_in_by_hand(&mut w, 9, "Gamer");
    let kat = make_account(&mut w, "Kat", PASSWORD, true);
    let jun = make_account(&mut w, "Jun", OTHER_PASSWORD, false);
    assert!(w.server.sign_up_closed());
    // Not an admin.
    let j = launcher_in(&mut w, 2, "Jun", Some(OTHER_PASSWORD));
    w.pcs[j].ask_account(AccountAsk::List).unwrap();
    signed_out(&mut w, jun);
    let j = launcher_in(&mut w, 2, "Jun", None);
    let remove = AccountAsk::Remove { account: kat };
    w.pcs[j].ask_account(remove).unwrap();
    signed_out(&mut w, jun);
    assert!(w.server.account(kat).is_some());
    // Badly signed.
    let k = launcher_in(&mut w, 1, "Kat", Some(PASSWORD));
    let forged = ToServer::Account {
        seq: 7,
        op: AccountOp::Remove { account: jun },
        sig: [7; 64],
    };
    w.send(k, forged);
    signed_out(&mut w, kat);
    assert!(w.server.account(jun).is_some());
    // Replayed, or numbered no higher than the last.
    let (mut conn, nonce, _) = launcher_by_hand(&mut w, 1, "KAT");
    ToServer::Prove(signature(1, &nonce)).send(&mut conn);
    conn.flush().unwrap();
    w.step();
    assert!(matches!(heard(&mut conn)[0], ToPc::Welcome(_)));
    let request = |seq: u32| {
        let op = AccountOp::List;
        let sig = key(1).sign(&live::account_proof(&nonce, seq, &op));
        let sig = sig.to_bytes();
        ToServer::Account { seq, op, sig }
    };
    request(1).send(&mut conn);
    conn.flush().unwrap();
    w.step();
    let answered = heard(&mut conn).into_iter().any(|m| match m {
        // Gamer, Jun and Kat.
        ToPc::Accounts(a) => a.answers == 1 && a.ok && a.rows.len() == 3,
        _ => false,
    });
    assert!(answered);
    request(1).send(&mut conn);
    conn.flush().unwrap();
    w.step();
    assert_eq!(w.server.pc_of(kat), None);
    let (mut conn, nonce, _) = launcher_by_hand(&mut w, 1, "KAT");
    ToServer::Prove(signature(1, &nonce)).send(&mut conn);
    let op = AccountOp::List;
    let sig = key(1).sign(&live::account_proof(&nonce, 0, &op)).to_bytes();
    ToServer::Account { seq: 0, op, sig }.send(&mut conn);
    conn.flush().unwrap();
    w.step();
    assert_eq!(w.server.pc_of(kat), None);
    // From the game, it's unexpected.
    let gamer = w.server.find("Gamer").unwrap();
    let op = AccountOp::List;
    let sig = [0; 64];
    ToServer::Account { seq: 1, op, sig }.send(&mut game);
    game.flush().unwrap();
    w.step();
    assert_eq!(w.server.pc_of(gamer), None);
}

#[test]
fn an_admin_creates_resets_signs_out_and_removes_from_the_launcher() {
    let mut w = closed("launcher-admin");
    make_account(&mut w, "Kat", PASSWORD, true);
    let admin = launcher_in(&mut w, 1, "Kat", Some(PASSWORD));
    // Gamertags and passwords keep the rules.
    let create = |gamertag: &str, password: &str| AccountAsk::Create {
        gamertag: gamertag.into(),
        password: secret(password),
    };
    for (gamertag, password, why) in [
        ("1JUN", "FIRST-PASS-1", password::GAMERTAG_RULES),
        ("kat", "FIRST-PASS-1", live::GAMERTAG_TAKEN),
        ("Jun", "SHORT", password::RULES),
    ] {
        let answer = ask(&mut w, admin, create(gamertag, password));
        assert_eq!((answer.ok, answer.text.as_str()), (false, why));
    }
    let answer = ask(&mut w, admin, create("jun", "first pass 1"));
    assert_eq!((answer.ok, answer.text.as_str()), (true, "CREATED JUN"));
    let jun = w.server.find("JUN").unwrap();
    assert!(answer
        .rows
        .iter()
        .any(|r| r.account == jun && r.gamertag == "JUN"));
    assert!(!w.server.logins.is_admin(jun));
    let j = launcher_in(&mut w, 2, "Jun", Some("FIRST-PASS-1"));
    // A new password: Jun's PC stays signed in.
    let set = AccountAsk::SetPassword {
        account: jun,
        password: secret("SECOND-PASS-2"),
    };
    let answer = ask(&mut w, admin, set);
    assert_eq!(
        answer.text,
        "JUN HAS A NEW PASSWORD. THEIR PCS STAY SIGNED IN."
    );
    w.run(1.0);
    assert!(w.pcs[j].signed_in());
    // Signed out of every PC: it's turned away now, and needs the new
    // password.
    let answer = ask(&mut w, admin, AccountAsk::SignOut { account: jun });
    assert_eq!(answer.text, "JUN IS SIGNED OUT OF ALL PCS.");
    assert_eq!(refusal(&mut w, j), live::SIGNED_OUT);
    let why = launcher_refused(&mut w, 2, "Jun", Some("FIRST-PASS-1"));
    assert_eq!(why, live::WRONG_PASSWORD);
    let j = launcher_in(&mut w, 2, "Jun", Some("SECOND-PASS-2"));
    // Removed.
    let answer = ask(&mut w, admin, AccountAsk::Remove { account: jun });
    assert_eq!((answer.ok, answer.text.as_str()), (true, "REMOVED JUN"));
    assert_eq!(refusal(&mut w, j), live::ACCOUNT_REMOVED);
    assert_eq!(w.server.find("JUN"), None);
    assert!(!answer.rows.iter().any(|r| r.account == jun));
    let answer = ask(&mut w, admin, AccountAsk::Remove { account: jun });
    assert_eq!((answer.ok, answer.text.as_str()), (false, NO_SUCH_ACCOUNT));
}

#[test]
fn a_launcher_admin_cannot_touch_their_own_account_or_another_admin() {
    let mut w = closed("admin-limits");
    let kat = make_account(&mut w, "Kat", PASSWORD, true);
    let noble = make_account(&mut w, "Noble", PASSWORD, true);
    let jun = make_account(&mut w, "Jun", PASSWORD, false);
    let admin = launcher_in(&mut w, 1, "Kat", Some(PASSWORD));
    let set = |account| AccountAsk::SetPassword {
        account,
        password: secret(OTHER_PASSWORD),
    };
    for (asked, why) in [
        (set(kat), YOUR_OWN),
        (AccountAsk::SignOut { account: kat }, YOUR_OWN),
        (AccountAsk::Remove { account: kat }, YOUR_OWN),
        (set(noble), AN_ADMINS),
        (AccountAsk::SignOut { account: noble }, AN_ADMINS),
        (AccountAsk::Remove { account: noble }, AN_ADMINS),
        (AccountAsk::SignOut { account: 0x1234 }, NO_SUCH_ACCOUNT),
    ] {
        let answer = ask(&mut w, admin, asked);
        assert_eq!((answer.ok, answer.text.as_str()), (false, why));
    }
    assert!(w.server.account(noble).is_some());
    // Only the command line makes admins, and renames.
    let by = Actor::Admin(kat);
    let phc = password::hash(PASSWORD, Cost::Cheap).unwrap();
    let add = Command::Add {
        gamertag: "Carter".into(),
        phc,
        admin: true,
    };
    assert_eq!(
        w.server.command(by, add, w.now),
        Err(ONLY_THE_COMMAND_LINE.into())
    );
    let promote = Command::SetAdmin {
        account: jun,
        admin: true,
    };
    assert_eq!(
        w.server.command(by, promote, w.now),
        Err(ONLY_THE_COMMAND_LINE.into())
    );
    let rename = Command::Rename {
        account: jun,
        gamertag: "Carter".into(),
    };
    assert_eq!(w.server.command(by, rename, w.now), Err(RENAMES.into()));
    let bad = Command::SetPassword {
        account: jun,
        phc: "hunter2".into(),
    };
    assert_eq!(w.server.command(by, bad, w.now), Err(NOT_A_HASH.into()));
    // But the rest of the accounts are theirs.
    let answer = ask(&mut w, admin, set(jun));
    assert!(answer.ok);
}

#[test]
fn signing_out_refuses_the_pc_now_and_its_next_sign_in_needs_the_password() {
    let mut w = closed("sign-out");
    let kat = make_account(&mut w, "Kat", PASSWORD, false);
    let i = launcher_in(&mut w, 1, "Kat", Some(PASSWORD));
    let done = w
        .server
        .command(Actor::Console, Command::SignOut { account: kat }, w.now);
    assert_eq!(done.as_deref(), Ok("KAT IS SIGNED OUT OF ALL PCS."));
    assert_eq!(refusal(&mut w, i), live::SIGNED_OUT);
    let why = launcher_refused(&mut w, 1, "Kat", None);
    assert_eq!(why, live::PASSWORD_NEEDED);
    let i = launcher_in(&mut w, 1, "Kat", Some(PASSWORD));
    assert_eq!(w.welcome(i).account, kat);
    // An older server's account has no password: the game is told it
    // was signed out, and so is whoever signs it out.
    let mut w = World::with_secret("sign-out-veteran", Some(SECRET));
    let veteran = veteran(&mut w);
    w.server.set_sign_up(SignUp::Closed);
    let a = w.sign_in(1, "Veteran");
    let signed_out = Command::SignOut {
        account: veteran.id,
    };
    let done = w.server.command(Actor::Console, signed_out, w.now);
    assert_eq!(
        done.as_deref(),
        Ok("VETERAN IS SIGNED OUT OF ALL PCS. GIVE THEM A PASSWORD SO THEY CAN SIGN IN AGAIN.")
    );
    assert_eq!(refusal(&mut w, a), live::SIGNED_OUT);
    let a = w.connect(1, "Veteran");
    assert_eq!(refusal(&mut w, a), live::SIGNED_OUT);
}

#[test]
fn a_new_password_keeps_pcs_signed_in_and_clears_the_lock() {
    let mut w = closed("new-password");
    let kat = make_account(&mut w, "Kat", PASSWORD, false);
    let i = launcher_in(&mut w, 1, "Kat", Some(PASSWORD));
    for n in 2..=6 {
        let why = launcher_refused(&mut w, n, "Kat", Some(OTHER_PASSWORD));
        assert_eq!(why, live::WRONG_PASSWORD);
    }
    let why = launcher_refused(&mut w, 7, "Kat", Some(PASSWORD));
    assert_eq!(why, live::TOO_MANY_TRIES);
    let phc = password::hash(OTHER_PASSWORD, Cost::Cheap).unwrap();
    let set = Command::SetPassword { account: kat, phc };
    let done = w.server.command(Actor::Console, set, w.now);
    assert_eq!(
        done.as_deref(),
        Ok("KAT HAS A NEW PASSWORD. THEIR PCS STAY SIGNED IN.")
    );
    w.run(1.0);
    assert!(w.pcs[i].signed_in());
    // Unlocked, with the new password only.
    let why = launcher_refused(&mut w, 8, "Kat", Some(PASSWORD));
    assert_eq!(why, live::WRONG_PASSWORD);
    let j = launcher_in(&mut w, 7, "Kat", Some(OTHER_PASSWORD));
    assert_eq!(w.welcome(j).account, kat);
}

#[test]
fn removing_drops_the_pc_purges_friends_frees_the_gamertag_and_its_card_is_ignored() {
    let mut w = closed("remove");
    let kat = make_account(&mut w, "Kat", PASSWORD, false);
    let jun = make_account(&mut w, "Jun", OTHER_PASSWORD, false);
    let a = launcher_in(&mut w, 1, "Kat", Some(PASSWORD));
    let b = launcher_in(&mut w, 2, "Jun", Some(OTHER_PASSWORD));
    // Friends.
    w.send(a, ToServer::FriendRequest("JUN".into()));
    let asked = |w: &World| w.pcs[b].view.friends.iter().any(|f| f.account == kat);
    w.until(asked);
    w.send(b, ToServer::FriendAccept(kat));
    let friends = |w: &World| {
        let list = &w.pcs[a].view.friends;
        list.iter()
            .any(|f| f.account == jun && f.relation == Relation::Friend)
    };
    w.until(friends);
    let jun_card = w.card(2);
    let done = w
        .server
        .command(Actor::Console, Command::Remove { account: jun }, w.now);
    assert_eq!(done.as_deref(), Ok("REMOVED JUN"));
    assert_eq!(refusal(&mut w, b), live::ACCOUNT_REMOVED);
    // Off Kat's friends list, and out of every file.
    w.until(|w| w.pcs[a].view.friends.is_empty());
    let id = format!("{jun:016x}");
    for file in ["accounts.txt", "friends.txt", "logins.txt"] {
        let text = std::fs::read_to_string(w.data.0.join(file)).unwrap_or_default();
        assert!(!text.contains(&id), "{file}: {text}");
    }
    assert_eq!(w.server.account(jun), None);
    // The gamertag is free: a new account takes it, and the old card
    // brings nothing back.
    let new_jun = make_account(&mut w, "Jun", OTHER_PASSWORD, false);
    assert_ne!(new_jun, jun);
    std::fs::write(w.card_path(2), &jun_card).unwrap();
    let b = launcher_in(&mut w, 2, "Jun", Some(OTHER_PASSWORD));
    assert_eq!(w.welcome(b).account, new_jun);
    assert_eq!(w.server.account(jun), None);
    // Once it's gone, its PC is no one's.
    w.server
        .command(Actor::Console, Command::Remove { account: new_jun }, w.now)
        .unwrap();
    let why = launcher_refused(&mut w, 2, "Jun", None);
    assert_eq!(why, live::PASSWORD_NEEDED);
}

#[test]
fn the_last_admin_cannot_be_demoted_or_removed() {
    let mut w = closed("last-admin");
    let kat = make_account(&mut w, "Kat", PASSWORD, true);
    let demote = |account| Command::SetAdmin {
        account,
        admin: false,
    };
    let console = Actor::Console;
    assert_eq!(
        w.server.command(console, demote(kat), w.now),
        Err(LAST_ADMIN.into())
    );
    let remove = Command::Remove { account: kat };
    assert_eq!(
        w.server.command(console, remove, w.now),
        Err(LAST_ADMIN.into())
    );
    let noble = make_account(&mut w, "Noble", PASSWORD, false);
    let promote = Command::SetAdmin {
        account: noble,
        admin: true,
    };
    let done = w.server.command(console, promote, w.now);
    assert_eq!(done.as_deref(), Ok("NOBLE IS AN ADMIN NOW."));
    let done = w.server.command(console, demote(kat), w.now);
    assert_eq!(done.as_deref(), Ok("KAT ISN'T AN ADMIN NOW."));
    assert_eq!(
        w.server.command(console, demote(noble), w.now),
        Err(LAST_ADMIN.into())
    );
    assert_eq!(w.server.logins.admins(), 1);
}

#[test]
fn change_password_needs_the_old_one_and_keeps_the_pcs() {
    let mut w = closed("change-password");
    let kat = make_account(&mut w, "Kat", PASSWORD, false);
    launcher_in(&mut w, 2, "Kat", Some(PASSWORD));
    let i = launcher_in(&mut w, 1, "Kat", Some(PASSWORD));
    let change = |old: &str, new: &str| AccountAsk::ChangePassword {
        old: secret(old),
        new: secret(new),
    };
    let answer = ask(&mut w, i, change(OTHER_PASSWORD, "THIRD-PASSWORD"));
    assert_eq!(
        (answer.ok, answer.text.as_str()),
        (false, WRONG_OLD_PASSWORD)
    );
    let answer = ask(&mut w, i, change(PASSWORD, "SHORT"));
    assert_eq!((answer.ok, answer.text.as_str()), (false, password::RULES));
    let answer = ask(&mut w, i, change(PASSWORD, "THIRD-PASSWORD"));
    assert_eq!((answer.ok, answer.text.as_str()), (true, PASSWORD_CHANGED));
    // Its PCs stay signed in, and only the new password signs in another.
    w.run(1.0);
    assert!(w.pcs[i].signed_in());
    let j = launcher_in(&mut w, 2, "Kat", None);
    assert_eq!(w.welcome(j).account, kat);
    let why = launcher_refused(&mut w, 3, "Kat", Some(PASSWORD));
    assert_eq!(why, live::WRONG_PASSWORD);
    // (A right one clears the wrong ones before it.)
    let k = launcher_in(&mut w, 4, "Kat", Some("third password"));
    // Wrong old passwords count towards the lock.
    for _ in 0..5 {
        let answer = ask(&mut w, k, change(PASSWORD, "FOURTH-PASSWORD"));
        assert_eq!(answer.text, WRONG_OLD_PASSWORD);
    }
    let answer = ask(&mut w, k, change("THIRD-PASSWORD", "FOURTH-PASSWORD"));
    assert_eq!(
        (answer.ok, answer.text.as_str()),
        (false, live::TOO_MANY_TRIES)
    );
    // An account with no password yet can't change it.
    let mut w = World::with_secret("change-no-password", Some(SECRET));
    veteran(&mut w);
    w.server.set_password_cost(Cost::Cheap);
    let i = launcher_in(&mut w, 1, "Veteran", None);
    let answer = ask(&mut w, i, change("ANYTHING-AT-ALL", "THIRD-PASSWORD"));
    assert_eq!((answer.ok, answer.text.as_str()), (false, NO_PASSWORD_YET));
}

#[test]
fn a_card_raises_only_the_account_signed_in_and_never_renames_it() {
    let mut w = World::with_secret("closed-card", Some(SECRET));
    let veteran = veteran(&mut w);
    w.server.set_sign_up(SignUp::Closed);
    w.server.set_password_cost(Cost::Cheap);
    let card_key = card::server_key(&w.data.0, Some(SECRET)).unwrap();
    let card = |gamertag: &str, seq, level| {
        let mut a = veteran.clone();
        a.gamertag = gamertag.into();
        a.seq = seq;
        a.stats[0].rank = Rank {
            xp: crate::levels::min_xp(level),
            level,
        };
        card::sign(&a, &card_key)
    };
    // A newer card of its own raises it, under the name the server has.
    std::fs::write(w.card_path(1), card("IMPOSTER", 9, 20)).unwrap();
    let a = w.sign_in(1, "Veteran");
    assert_eq!(w.welcome(a).best, 20);
    assert_eq!(w.welcome(a).gamertag, "VETERAN");
    // Shown by another account's PC, it does nothing.
    let kat = make_account(&mut w, "Kat", PASSWORD, false);
    std::fs::write(w.card_path(2), card("VETERAN", 30, 40)).unwrap();
    let b = launcher_in(&mut w, 2, "Kat", Some(PASSWORD));
    assert_eq!(w.welcome(b).account, kat);
    assert_eq!(w.welcome(b).best, 1);
    assert_eq!(w.server.account(veteran.id).unwrap().best_level(), 20);
    assert_eq!(w.server.account(veteran.id).unwrap().gamertag, "VETERAN");
}

#[test]
fn a_card_never_makes_an_account_when_closed() {
    let mut w = World::with_secret("closed-card-wipe", Some(SECRET));
    let veteran = veteran(&mut w);
    w.sign_in(1, "Veteran");
    w.wipe();
    w.server.set_sign_up(SignUp::Closed);
    let a = w.connect(1, "Veteran");
    assert_eq!(refusal(&mut w, a), live::NO_ACCOUNT);
    let why = launcher_refused(&mut w, 1, "Veteran", None);
    assert_eq!(why, live::PASSWORD_NEEDED);
    assert_eq!(w.server.account(veteran.id), None);
    assert_eq!(w.accounts_txt(), "");
}

#[test]
fn logins_and_profiles_never_rename_when_closed_or_for_password_accounts() {
    let mut w = closed("no-renames");
    let kat = make_account(&mut w, "Kat", PASSWORD, false);
    launcher_in(&mut w, 1, "Kat", Some(PASSWORD));
    let i = launcher_in(&mut w, 1, "Someone", None);
    assert_eq!(w.welcome(i).gamertag, "KAT");
    w.send(
        i,
        ToServer::Profile {
            gamertag: "Someone".into(),
            look: Look::default_for(7),
        },
    );
    w.until(|w| !w.notices(i).is_empty());
    assert_eq!(w.notices(i), [live::ASK_TO_RENAME]);
    // The look is taken all the same.
    w.until(|w| w.server.account(kat).unwrap().look == Look::default_for(7));
    assert_eq!(w.server.account(kat).unwrap().gamertag, "KAT");
    // The same gamertag, in another case, is no rename.
    w.send(
        i,
        ToServer::Profile {
            gamertag: "kat".into(),
            look: Look::default_for(8),
        },
    );
    w.until(|w| w.server.account(kat).unwrap().look == Look::default_for(8));
    assert_eq!(w.notices(i).len(), 1);
    // Open, an account with a password still isn't renamed by its PC.
    let mut w = World::with_secret("no-renames-open", Some(SECRET));
    w.server.set_password_cost(Cost::Cheap);
    let kat = make_account(&mut w, "Kat", PASSWORD, false);
    assert!(!w.server.sign_up_closed());
    launcher_in(&mut w, 1, "Kat", Some(PASSWORD));
    let i = launcher_in(&mut w, 1, "Someone", None);
    assert_eq!(w.welcome(i).gamertag, "KAT");
    w.send(
        i,
        ToServer::Profile {
            gamertag: "Someone".into(),
            look: Look::default(),
        },
    );
    w.until(|w| !w.notices(i).is_empty());
    assert_eq!(w.notices(i), [live::ASK_TO_RENAME]);
    assert_eq!(w.server.account(kat).unwrap().gamertag, "KAT");
    // While one without, from an open server's new PC, is.
    let j = w.sign_in(2, "Carter");
    w.send(
        j,
        ToServer::Profile {
            gamertag: "Emile".into(),
            look: Look::default(),
        },
    );
    w.until(|w| w.welcome(j).gamertag == "EMILE");
    // And closed, an older server's account keeps its name too.
    let mut w = World::with_secret("no-renames-veteran", Some(SECRET));
    veteran(&mut w);
    w.server.set_sign_up(SignUp::Closed);
    let a = w.sign_in(1, "New Name");
    assert_eq!(w.welcome(a).gamertag, "VETERAN");
}

#[test]
fn the_default_is_closed_once_there_is_an_admin() {
    let mut w = World::new("auto-sign-up");
    w.server.set_password_cost(Cost::Cheap);
    assert!(!w.server.sign_up_closed());
    assert!(w.server.sign_up_line().starts_with("sign-up: open"));
    let a = w.sign_in(1, "Carter");
    let carter = w.welcome(a).account;
    // An admin closes it...
    let promote = Command::SetAdmin {
        account: carter,
        admin: true,
    };
    w.server.command(Actor::Console, promote, w.now).unwrap();
    assert!(w.server.sign_up_closed());
    assert!(w.server.sign_up_line().starts_with("sign-up: closed"));
    let b = w.connect(2, "Emile");
    assert_eq!(refusal(&mut w, b), live::NO_ACCOUNT);
    // ...and stays so after a restart.
    w.restart();
    assert!(w.server.sign_up_closed());
    // Told to be open, it is, and says so.
    w.server.set_sign_up(SignUp::Open);
    assert!(w.server.sign_up_line().contains("warning: sign-up is open"));
    let b = w.sign_in(2, "Emile");
    assert!(w.pcs[b].signed_in());
    // Told to be closed with no admins, it says how to make one.
    let mut w = World::new("closed-no-admins");
    w.server.set_sign_up(SignUp::Closed);
    let line = w.server.sign_up_line();
    assert!(line.contains("no admins yet"), "{line}");
}

#[test]
fn every_launcher_hears_accounts_after_welcome_with_rows_only_for_admins() {
    let mut w = closed("accounts-rows");
    let kat = make_account(&mut w, "Kat", PASSWORD, true);
    let jun = make_account(&mut w, "Jun", OTHER_PASSWORD, false);
    let a = launcher_in(&mut w, 1, "Kat", Some(PASSWORD));
    let b = launcher_in(&mut w, 2, "Jun", Some(OTHER_PASSWORD));
    for i in [a, b] {
        let welcomed = w.events[i].iter().position(|e| *e == LiveEvent::Welcomed);
        let listed = w.events[i].iter().position(|e| *e == LiveEvent::Accounts);
        assert!(welcomed.unwrap() < listed.unwrap());
    }
    w.run(1.0);
    let admin = accounts(&w, a).clone();
    assert!(admin.admin && admin.password);
    assert_eq!((admin.answers, admin.total), (0, 2));
    let tags: Vec<&str> = admin.rows.iter().map(|r| r.gamertag.as_str()).collect();
    assert_eq!(tags, ["JUN", "KAT"]);
    let row = admin.rows.iter().find(|r| r.account == kat).unwrap();
    assert_eq!(row.flags, ROW_ADMIN | ROW_PASSWORD | ROW_ONLINE);
    assert_eq!((row.pcs, row.best), (1, 1));
    let player = accounts(&w, b).clone();
    assert!(!player.admin && player.password);
    assert_eq!(player.total, 2);
    assert_eq!(player.rows.len(), 1);
    assert_eq!(player.rows[0].account, jun);
    assert_eq!(player.rows[0].flags, ROW_PASSWORD | ROW_ONLINE);
    // An account changes: both hear.
    make_account(&mut w, "Noble", PASSWORD, false);
    w.until(|w| accounts(w, a).total == 3 && accounts(w, b).total == 3);
    assert_eq!(accounts(&w, a).rows.len(), 3);
    assert_eq!(accounts(&w, b).rows.len(), 1);
    // A launcher that doesn't know the server's key is told it changed
    // and gives up before sending anything; one that knows it signs in.
    let i = connect_launcher(&mut w, 1, "Kat", None);
    w.pcs[i].set_access(Access {
        pin: Some([1; 32]),
        ..Access::default()
    });
    w.until(|w| !w.events[i].is_empty());
    let now = w.server.identity_key();
    assert_eq!(w.events[i], [LiveEvent::KeyChanged { was: [1; 32], now }]);
    let i = connect_launcher(&mut w, 1, "Kat", None);
    w.pcs[i].set_access(Access {
        pin: Some(now),
        ..Access::default()
    });
    w.until(|w| w.pcs[i].signed_in());
}

#[test]
fn a_launcher_gives_up_on_a_server_that_does_not_prove_its_key() {
    let mut w = World::new("unproven");
    // A "server" that sends only a challenge.
    let (mut server_end, pc_end) = Connection::pair();
    let card = w.card_path(1);
    let mut pc = LiveClient::launcher(pc_end, key(1), &profile("Kat"), "test", &card, 0.0);
    let motd = String::new();
    ToPc::Challenge {
        nonce: [5; 32],
        motd,
    }
    .send(&mut server_end);
    server_end.flush().unwrap();
    let events = pc.poll(0.1);
    assert_eq!(events, [LiveEvent::Lost(SERVER_UNPROVEN.into())]);
    // Nothing was sent after the LOGIN.
    let sent = server_end.receive().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, kind::LOGIN);
    // Nor with a key signed by someone else.
    let (mut server_end, pc_end) = Connection::pair();
    let mut pc = LiveClient::launcher(pc_end, key(1), &profile("Kat"), "test", &card, 0.0);
    let seal_key = seal::public(&[6; 32]);
    let sig = key(9)
        .sign(&seal::key_proof(&[5; 32], &seal_key))
        .to_bytes();
    let server = key(8).verifying_key().to_bytes();
    ToPc::SealKey {
        key: seal_key,
        server,
        sig,
    }
    .send(&mut server_end);
    let motd = String::new();
    ToPc::Challenge {
        nonce: [5; 32],
        motd,
    }
    .send(&mut server_end);
    server_end.flush().unwrap();
    assert_eq!(pc.poll(0.1), [LiveEvent::Lost(SERVER_UNPROVEN.into())]);
    assert_eq!(server_end.receive().unwrap().len(), 1);
    w.run(0.1);
}

#[test]
fn a_login_line_without_an_account_is_dropped_and_an_account_without_one_is_adopted() {
    let mut w = World::with_secret("adopt", Some(SECRET));
    let phc = password::hash(PASSWORD, Cost::Cheap).unwrap();
    let stray = format!(
        "u 00000000deadbeef 1 {phc}\nd 00000000deadbeef {} 5 STRAY PC\n",
        "07".repeat(32)
    );
    std::fs::write(w.data.0.join("logins.txt"), stray).unwrap();
    let veteran = veteran(&mut w);
    let text = std::fs::read_to_string(w.data.0.join("logins.txt")).unwrap();
    assert!(!text.contains("deadbeef"), "{text}");
    assert!(
        text.contains(&format!("u {:016x} 0 -\n", veteran.id)),
        "{text}"
    );
    assert!(text.contains(" OLD PC\n"), "{text}");
    // So the stray admin doesn't close sign-up.
    assert!(!w.server.sign_up_closed());
    let a = w.sign_in(1, "Veteran");
    assert_eq!(w.welcome(a).account, veteran.id);
}

#[test]
fn admin_requests_parse_and_answer() {
    let mut w = closed("admin-requests");
    let answer = |w: &mut World, request: Request| {
        // Through its line, as the admin socket takes it.
        let request = Request::parse(&request.line()).unwrap();
        admin::answer(&mut w.server, request, w.now)
    };
    let phc = password::hash(PASSWORD, Cost::Cheap).unwrap();
    let add = |gamertag: &str, admin| Request::Add {
        gamertag: gamertag.into(),
        phc: phc.clone(),
        admin,
    };
    assert_eq!(answer(&mut w, add("Kat", true)), ["ok\tCREATED KAT"]);
    assert_eq!(
        answer(&mut w, add("KAT", false)),
        [format!("error\t{}", live::GAMERTAG_TAKEN)]
    );
    assert_eq!(answer(&mut w, add("Jun 1", false)), ["ok\tCREATED JUN 1"]);
    let bad = Request::Passwd {
        gamertag: "Kat".into(),
        phc: "hunter2".into(),
    };
    assert_eq!(answer(&mut w, bad), [format!("error\t{NOT_A_HASH}")]);
    let kat = w.server.find("kat").unwrap();
    launcher_in(&mut w, 1, "Kat", Some(PASSWORD));
    let created = w.server.account(kat).unwrap().created;
    let list = answer(&mut w, Request::List);
    let jun = w.server.find("JUN 1").unwrap();
    let jun_created = w.server.account(jun).unwrap().created;
    let added = w.server.devices_of(kat)[0].added;
    assert_eq!(
        list,
        [
            format!("row\t{jun:016x}\tJUN 1\t0\t1\t0\t0\t1\t{jun_created}"),
            format!("row\t{kat:016x}\tKAT\t1\t1\t1\t1\t1\t{created}"),
            format!("pc\t{kat:016x}\t{added}\tPC 1"),
            "ok\t2 ACCOUNTS".to_string(),
        ]
    );
    let key = w.server.identity_key();
    assert_eq!(
        answer(&mut w, Request::Key),
        [format!(
            "ok\t{}\t{}",
            crate::store::hex(&key),
            seal::short_code(&key)
        )]
    );
    let unknown = Request::Revoke {
        gamertag: "Nobody".into(),
    };
    assert_eq!(
        answer(&mut w, unknown),
        ["error\tNO ACCOUNT IS CALLED NOBODY"]
    );
    let rename = Request::Rename {
        gamertag: "jun 1".into(),
        new: "Jun".into(),
    };
    assert_eq!(answer(&mut w, rename), ["ok\tJUN 1 IS JUN NOW"]);
    let promote = Request::Admin {
        gamertag: "Jun".into(),
        on: true,
    };
    let done = answer(&mut w, promote);
    assert_eq!(done, ["ok\tJUN IS AN ADMIN NOW."]);
    let passwd = Request::Passwd {
        gamertag: "Jun".into(),
        phc: phc.clone(),
    };
    let done = answer(&mut w, passwd);
    assert_eq!(
        done,
        ["ok\tJUN HAS A NEW PASSWORD. THEIR PCS STAY SIGNED IN."]
    );
    let revoke = Request::Revoke {
        gamertag: "Kat".into(),
    };
    assert_eq!(
        answer(&mut w, revoke),
        ["ok\tKAT IS SIGNED OUT OF ALL PCS."]
    );
    let remove = Request::Remove {
        gamertag: "Kat".into(),
    };
    assert_eq!(answer(&mut w, remove), ["ok\tREMOVED KAT"]);
    let remove = Request::Remove {
        gamertag: "Jun".into(),
    };
    assert_eq!(answer(&mut w, remove), [format!("error\t{LAST_ADMIN}")]);
    assert_eq!(
        answer(&mut w, Request::List).last().unwrap(),
        "ok\t1 ACCOUNT"
    );
}
