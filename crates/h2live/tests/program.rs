//! The h2live program itself, as players' PCs find it: over TCP on this
//! PC, with web requests and WebSockets on the one port; and its command
//! line (`h2live user ...`), with the program running and without. (Unix
//! only: it's stopped as Ctrl+C would, with a signal.)

#![cfg(unix)]

use ed25519_dalek::SigningKey;
use h2live::client::{Access, LiveClient, LiveEvent, Profile};
use h2live::password::Secret;
use h2net::live::{self, ToServer};
use h2relay::{ClientConfig, Failure, Refusal, RelayClient, State};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// How long anything may take.
const WAIT: Duration = Duration::from_secs(10);

/// A folder of the test's own, removed afterwards.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!("h2live-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The program, running, and the lines it has said.
struct Program {
    child: Child,
    lines: Receiver<String>,
    said: Vec<String>,
    port: u16,
}

impl Program {
    /// Start it on any free port (as a host would give it one, so it
    /// leaves the router alone), keeping its accounts in `data`.
    fn start(data: &Path) -> Program {
        Program::start_with(data, &[])
    }

    /// `start`, with more of the environment (`H2LIVE_SIGNUP`, say).
    fn start_with(data: &Path, env: &[(&str, &str)]) -> Program {
        let (mut program, stdout) = Program::launch_with(data, env);
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in stdout.lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        });
        program.lines = lines;
        program
    }

    /// Start it, leaving what it says once it has said which port it's on
    /// to the caller to read (or not).
    fn launch(data: &Path) -> (Program, BufReader<ChildStdout>) {
        Program::launch_with(data, &[])
    }

    fn launch_with(data: &Path, env: &[(&str, &str)]) -> (Program, BufReader<ChildStdout>) {
        let mut child = program(data, env).stdout(Stdio::piped()).spawn().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        while !line.contains("listening on port ") {
            line.clear();
            let read = stdout.read_line(&mut line).unwrap();
            assert!(read > 0, "it never said which port it's on");
        }
        let after = line.split("listening on port ").nth(1).unwrap();
        let port = after.split(' ').next().unwrap().parse().unwrap();
        let program = Program {
            child,
            lines: mpsc::channel().1,
            said: Vec::new(),
            port,
        };
        (program, stdout)
    }

    /// The first line it said with `what` in it, once it has.
    fn wait_for(&mut self, what: &str) -> String {
        let start = Instant::now();
        loop {
            if let Some(line) = self.said.iter().find(|l| l.contains(what)) {
                return line.clone();
            }
            let left = WAIT.saturating_sub(start.elapsed());
            match self.lines.recv_timeout(left) {
                Ok(line) => self.said.push(line),
                Err(_) => panic!("it never said {what:?}: {:#?}", self.said),
            }
        }
    }

    /// The whole answer to a web request for `path`.
    fn get(&self, path: &str) -> String {
        self.ask(&format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n"))
    }

    /// The whole answer to `request`.
    fn ask(&self, request: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        s.set_read_timeout(Some(WAIT)).unwrap();
        s.write_all(request.as_bytes()).unwrap();
        let mut answer = String::new();
        s.read_to_string(&mut answer).unwrap();
        answer
    }

    /// The answer to asking for a WebSocket at `path`.
    fn ask_websocket(&self, path: &str) -> String {
        self.ask(&format!(
            "GET {path} HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\
             Connection: Upgrade\r\nSec-WebSocket-Version: 13\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
        ))
    }

    /// Stop it as Ctrl+C does, and wait until it has.
    fn stop(&mut self) {
        let pid = self.child.id().to_string();
        let kill = Command::new("kill").args(["-INT", &pid]).status();
        assert!(kill.unwrap().success());
        let start = Instant::now();
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(start.elapsed() < WAIT, "still running");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(status.success(), "{status}");
        self.said.extend(self.lines.iter());
    }
}

impl Drop for Program {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The program, to run on any free port with its accounts in `data`, and
/// `env` (which `H2LIVE_SIGNUP`, say, is unset unless it names).
fn program(data: &Path, env: &[(&str, &str)]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_h2live"));
    command
        .env("PORT", "0")
        .env("H2LIVE_DATA", data)
        .env_remove("H2LIVE_SECRET")
        .env_remove("H2LIVE_SIGNUP");
    command.envs(env.iter().copied());
    command
}

/// What the command line did: its exit code, and what it said on the
/// standard output and error.
struct Ran {
    code: i32,
    out: String,
    err: String,
}

/// Run the command line (`h2live` with `args`) on the data folder `data`,
/// giving it `stdin`.
fn h2live(data: &Path, args: &[&str], stdin: &str) -> Ran {
    let mut child = program(data, &[])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    Ran {
        code: output.status.code().unwrap(),
        out: String::from_utf8(output.stdout).unwrap(),
        err: String::from_utf8(output.stderr).unwrap(),
    }
}

/// A password, as tests type it, and a wrong one.
const PASSWORD: &str = "K7MP-4QR9-XW2T";
const WRONG: &str = "Q9W8-E7R6-T5Y4";

/// The program never said a hash, nor any of `passwords`, as typed or as
/// sealed (`password::normalize`).
fn said_nothing_secret(program: &Program, passwords: &[&str]) {
    for line in &program.said {
        assert!(!line.contains("argon2"), "{line}");
        for p in passwords {
            let sealed = h2live::password::normalize(p);
            assert!(!line.contains(p) && !line.contains(&sealed), "{line}");
        }
    }
}

/// Bytes as lowercase hex.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// PCs signed in to the program, polled as games poll them (every frame),
/// on a clock of their own.
struct Pcs {
    pcs: Vec<LiveClient>,
    /// What each heard, in order.
    events: Vec<Vec<LiveEvent>>,
    start: Instant,
    home: TempDir,
}

impl Pcs {
    fn new() -> Pcs {
        Pcs {
            pcs: Vec::new(),
            events: Vec::new(),
            start: Instant::now(),
            home: TempDir::new("program-home"),
        }
    }

    /// PC number `n` dials the program and signs in as `gamertag`. Its
    /// index.
    fn connect(&mut self, port: u16, n: u8, gamertag: &str) -> usize {
        let url = format!("ws://127.0.0.1:{port}/live");
        let conn = h2net::dial(&url, WAIT).recv().unwrap().unwrap();
        let profile = Profile {
            gamertag: gamertag.into(),
            maps: vec![("lockout".into(), 1)],
            ..Profile::default()
        };
        let card = self.home.0.join(format!("card-{n}.txt"));
        let key = SigningKey::from_bytes(&[n; 32]);
        let now = self.start.elapsed().as_secs_f64();
        self.pcs
            .push(LiveClient::new(conn, key, &profile, &card, now));
        self.events.push(Vec::new());
        self.pcs.len() - 1
    }

    /// PC number `n` dials the program as a launcher and signs in as
    /// `gamertag`, with `password` if any. Its index.
    fn connect_launcher(
        &mut self,
        port: u16,
        n: u8,
        gamertag: &str,
        password: Option<&str>,
    ) -> usize {
        let url = format!("ws://127.0.0.1:{port}/live");
        let conn = h2net::dial(&url, WAIT).recv().unwrap().unwrap();
        let profile = Profile {
            gamertag: gamertag.into(),
            ..Profile::default()
        };
        let card = self.home.0.join(format!("card-{n}.txt"));
        let key = SigningKey::from_bytes(&[n; 32]);
        let now = self.start.elapsed().as_secs_f64();
        let mut pc = LiveClient::launcher(conn, key, &profile, "test", &card, now);
        pc.set_access(Access {
            pin: None,
            password: password.map(Secret::new),
            computer: format!("PC {n}"),
        });
        self.pcs.push(pc);
        self.events.push(Vec::new());
        self.pcs.len() - 1
    }

    /// PC `i` was welcomed, or turned away (why), once it is.
    fn answer(&mut self, i: usize) -> Result<(), String> {
        let answered = |e: &LiveEvent| matches!(e, LiveEvent::Welcomed | LiveEvent::Refused(_));
        self.until(|p| p.heard(i, answered));
        match self.events[i].iter().find(|e| answered(e)) {
            Some(LiveEvent::Refused(why)) => Err(why.clone()),
            _ => Ok(()),
        }
    }

    /// Poll every PC until `done`.
    fn until(&mut self, done: impl Fn(&Pcs) -> bool) {
        while !done(self) {
            assert!(self.start.elapsed() < 3 * WAIT, "{:#?}", self.events);
            let now = self.start.elapsed().as_secs_f64();
            for (pc, events) in self.pcs.iter_mut().zip(&mut self.events) {
                events.extend(pc.poll(now));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn party(&self, i: usize) -> Option<(u64, usize)> {
        let party = self.pcs[i].view.party.as_ref()?;
        Some((party.id, party.members.len()))
    }

    fn heard(&self, i: usize, what: impl Fn(&LiveEvent) -> bool) -> bool {
        self.events[i].iter().any(what)
    }
}

#[test]
fn the_program_serves_pcs_and_web_requests_and_stops_cleanly() {
    let data = TempDir::new("program-data");
    let mut program = Program::start(&data.0);

    // Web requests: a host's health check, the server's page, and
    // anything else.
    let health = program.get("/health");
    assert!(health.starts_with("HTTP/1.1 200 OK\r\n"), "{health}");
    assert!(health.ends_with("\r\n\r\nOK"), "{health}");
    let page = program.get("/");
    assert!(
        page.contains("\r\n\r\nH2LIVE\n0 PLAYERS ONLINE\n"),
        "{page}"
    );
    assert!(page.contains("\n0 MATCHES IN PROGRESS\n"), "{page}");
    let elsewhere = program.get("/admin");
    assert!(elsewhere.starts_with("HTTP/1.1 404 Not Found\r\n"));
    let elsewhere = program.ask_websocket("/admin");
    assert!(
        elsewhere.starts_with("HTTP/1.1 404 Not Found\r\n"),
        "{elsewhere}"
    );

    // Two PCs sign in over WebSockets, and one invites the other into its
    // party.
    let mut pcs = Pcs::new();
    let alpha = pcs.connect(program.port, 1, "ALPHA");
    let bravo = pcs.connect(program.port, 2, "BRAVO");
    pcs.until(|p| (0..2).all(|i| p.pcs[i].signed_in() && p.party(i).is_some()));
    let line = program.wait_for("live: ALPHA signed in from 127.0.0.1");
    // Each line says when, as "2026-10-04 12:34:56".
    let (date, time) = (&line[..10], &line[11..19]);
    assert_eq!(date.split('-').count(), 3, "{line}");
    assert_eq!(time.split(':').count(), 3, "{line}");
    let page = program.get("/");
    assert!(page.contains("\n2 PLAYERS ONLINE\n"), "{page}");
    let to = pcs.pcs[bravo].account().unwrap();
    pcs.pcs[alpha].send(ToServer::Invite(to));
    pcs.until(|p| !p.pcs[bravo].view.invites.is_empty());
    let (party, _) = pcs.party(alpha).unwrap();
    assert_eq!(pcs.pcs[bravo].view.invites[0], (party, "ALPHA".into()));
    pcs.pcs[bravo].send(ToServer::Accept(party));
    pcs.until(|p| (0..2).all(|i| p.party(i) == Some((party, 2))));

    // Ctrl+C: the PCs lose the server, and the accounts are on disk.
    program.stop();
    assert!(program.said.iter().any(|l| l.contains("stopped")));
    pcs.until(|p| (0..2).all(|i| p.heard(i, |e| matches!(e, LiveEvent::Lost(_)))));
    let accounts = std::fs::read_to_string(data.0.join("accounts.txt")).unwrap();
    assert!(accounts.contains(" ALPHA ") && accounts.contains(" BRAVO "));

    // Started again, it remembers them: ALPHA is still taken.
    let mut program = Program::start(&data.0);
    let mut pcs = Pcs::new();
    let charlie = pcs.connect(program.port, 3, "alpha");
    pcs.until(|p| !p.events[charlie].is_empty());
    let refused = LiveEvent::Refused(live::GAMERTAG_TAKEN.into());
    assert_eq!(pcs.events[charlie], [refused]);
    program.stop();
}

#[test]
fn the_program_runs_the_relay_on_udp_for_its_own_rooms_only() {
    let data = TempDir::new("relay-data");
    let mut program = Program::start(&data.0);
    // On UDP, the same port number as the sign-ins.
    let line = program.wait_for("relay on UDP port");
    let port = program.port;
    assert!(
        line.contains(&format!("relay on UDP port {port} ")),
        "{line}"
    );
    assert!(line.contains("only rooms h2live issues"), "{line}");
    // It answers (a challenge, then a refusal), but admits no room it
    // didn't issue: it's no open relay.
    let relay = SocketAddr::from(([127, 0, 0, 1], port));
    let config = ClientConfig {
        hello_timeout: Duration::from_millis(500),
        ..ClientConfig::default()
    };
    let stranger = RelayClient::connect_with(relay, 7, 1, config).unwrap();
    let start = Instant::now();
    while stranger.state() == State::Joining {
        assert!(start.elapsed() < WAIT, "no answer");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        stranger.state(),
        State::Failed(Failure::Refused(Refusal::NoSuchRoom))
    );
    assert!(stranger.stats().server_rtt.is_none());
    program.stop();
    assert!(!program
        .said
        .iter()
        .any(|l| l.contains("room 0000000000000007")));
}

#[test]
fn the_program_goes_on_while_no_one_reads_what_it_says() {
    let data = TempDir::new("unread-data");
    let (program, _unread) = Program::launch(&data.0);
    // Far more to say than a pipe holds: WebSockets asked for where there
    // are none, each said with its long path.
    let path = format!("/{}", "x".repeat(8000));
    for _ in 0..40 {
        let answer = program.ask_websocket(&path);
        assert!(answer.starts_with("HTTP/1.1 404 Not Found\r\n"), "{answer}");
    }
    let health = program.get("/health");
    assert!(health.ends_with("\r\n\r\nOK"), "{health}");
}

#[test]
fn the_program_goes_on_when_what_it_says_has_nowhere_to_go() {
    let data = TempDir::new("unheard-data");
    let (mut program, stdout) = Program::launch(&data.0);
    drop(stdout);
    // Something to say.
    program.ask_websocket("/admin");
    std::thread::sleep(Duration::from_millis(200));
    let health = program.get("/health");
    assert!(health.ends_with("\r\n\r\nOK"), "{health}");
    assert!(program.child.try_wait().unwrap().is_none());
}

/// The program's CPU time so far (Linux says).
#[cfg(target_os = "linux")]
fn cpu_time(program: &Program) -> Duration {
    let stat = std::fs::read_to_string(format!("/proc/{}/stat", program.child.id())).unwrap();
    // After the name, in brackets: user and system time are the 12th and
    // 13th fields, in clock ticks.
    let fields: Vec<&str> = stat
        .rsplit(')')
        .next()
        .unwrap()
        .split_whitespace()
        .collect();
    let ticks: u64 = fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap();
    let hz = Command::new("getconf").arg("CLK_TCK").output().unwrap();
    let hz: u64 = String::from_utf8(hz.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    Duration::from_secs_f64(ticks as f64 / hz as f64)
}

#[cfg(target_os = "linux")]
#[test]
fn the_program_rests_while_pcs_do_nothing() {
    let data = TempDir::new("program-rests");
    let program = Program::start(&data.0);
    let mut pcs = Pcs::new();
    for (n, gamertag) in ["ALPHA", "BRAVO", "CHARLIE", "DELTA"].iter().enumerate() {
        pcs.connect(program.port, n as u8 + 1, gamertag);
    }
    pcs.until(|p| p.pcs.iter().all(LiveClient::signed_in));
    // Four PCs in their lobbies a few seconds, pinging now and then: it
    // waits for them, not looking every moment.
    let (start, used) = (Instant::now(), cpu_time(&program));
    pcs.until(|_| start.elapsed() >= Duration::from_secs(4));
    let used = cpu_time(&program) - used;
    assert!(used < Duration::from_millis(40), "{used:?} in 4 s");
    // And answers at once.
    let pings: Vec<u32> = pcs.pcs.iter().filter_map(|pc| pc.view.round_trip).collect();
    assert!(
        pings.len() == 4 && pings.iter().all(|&ms| ms < 20),
        "{pings:?}"
    );
}

#[test]
fn accounts_are_made_from_the_command_line_while_the_program_runs() {
    let data = TempDir::new("cli-running");
    let mut program = Program::start_with(&data.0, &[("H2LIVE_SIGNUP", "closed")]);
    program.wait_for("sign-up: closed");
    program.wait_for("no admins yet");
    // The admin socket (open by now) is the folder owner's alone: nothing
    // else checks who connects to it. (Under any usual umask, a socket
    // left as bound would be 0755 or wider.)
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(data.0.join("admin.sock"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "admin.sock is {:o}", mode & 0o777);
    assert!(!data.0.join("admin.sock.tmp").exists());
    let ran = h2live(
        &data.0,
        &["user", "add", "kat", "--password-stdin"],
        &format!("{PASSWORD}\n"),
    );
    assert_eq!(
        (ran.code, ran.out.as_str()),
        (0, "KAT created.\n"),
        "{}",
        ran.err
    );
    assert!(ran.err.contains("hashing took "), "{}", ran.err);
    // The program made it (and said so).
    program.wait_for("live: command line: made the account KAT");
    let mut pcs = Pcs::new();
    let a = pcs.connect_launcher(program.port, 1, "Kat", Some(PASSWORD));
    assert_eq!(pcs.answer(a), Ok(()));
    let kat = pcs.pcs[a].account().unwrap();
    // That PC needs no password now.
    let b = pcs.connect_launcher(program.port, 1, "Kat", None);
    assert_eq!(pcs.answer(b), Ok(()));
    assert_eq!(pcs.pcs[b].account(), Some(kat));
    // Another does.
    let c = pcs.connect_launcher(program.port, 2, "Kat", None);
    assert_eq!(pcs.answer(c), Err(live::PASSWORD_NEEDED.into()));
    // A wrong password, and a password for a gamertag no one has.
    let d = pcs.connect_launcher(program.port, 3, "Kat", Some(WRONG));
    assert_eq!(pcs.answer(d), Err(live::WRONG_PASSWORD.into()));
    let e = pcs.connect_launcher(program.port, 4, "Nobody", Some(PASSWORD));
    assert_eq!(pcs.answer(e), Err(live::WRONG_PASSWORD.into()));
    let ran = h2live(&data.0, &["user", "list"], "");
    assert_eq!(ran.code, 0, "{}", ran.err);
    let row = ran.out.lines().find(|l| l.starts_with("KAT ")).unwrap();
    assert!(row.contains(" 1/5 "), "{}", ran.out);
    assert!(ran.out.contains("PC \"PC 1\""), "{}", ran.out);
    assert!(ran.out.ends_with("1 account\n"), "{}", ran.out);
    // The key a launcher checks the server by, as the owner builds it in.
    let key = pcs.pcs[a].view.server_key.unwrap();
    let shown = format!(
        "identity key: {}\nshort code:   {}\n",
        hex(&key),
        h2live::seal::short_code(&key)
    );
    let ran = h2live(&data.0, &["user", "key"], "");
    assert_eq!(
        (ran.code, ran.out.as_str()),
        (0, shown.as_str()),
        "{}",
        ran.err
    );
    program.stop();
    // Nothing secret was said.
    said_nothing_secret(&program, &[PASSWORD, WRONG]);
    // The same key with the program stopped (the folder opened here).
    let ran = h2live(&data.0, &["user", "key"], "");
    assert_eq!(
        (ran.code, ran.out.as_str()),
        (0, shown.as_str()),
        "{}",
        ran.err
    );
    // The socket goes with the program.
    assert!(!data.0.join("admin.sock").exists());
}

#[test]
fn a_second_program_on_the_same_folder_stops() {
    let data = TempDir::new("cli-second");
    let mut first = Program::start(&data.0);
    // (Said once its admin socket is open.)
    first.wait_for("sign-up: ");
    let second = program(&data.0, &[]).output().unwrap();
    assert!(!second.status.success());
    let said = String::from_utf8(second.stdout).unwrap();
    assert!(said.contains("FAILED: another h2live is using "), "{said}");
    // The first goes on, its command line too: the second never touched
    // its admin socket.
    let health = first.get("/health");
    assert!(health.ends_with("\r\n\r\nOK"), "{health}");
    assert!(data.0.join("admin.sock").exists());
    let ran = h2live(&data.0, &["user", "list"], "");
    assert_eq!(ran.code, 0, "{}", ran.err);
    first.stop();
}

#[test]
fn the_command_line_works_on_its_own_while_the_program_is_stopped() {
    let data = TempDir::new("cli-stopped");
    let ran = h2live(&data.0, &["user", "add", "Kat", "--admin"], "");
    assert_eq!(ran.code, 0, "{}", ran.err);
    let lines: Vec<&str> = ran.out.lines().collect();
    assert_eq!(lines.len(), 2, "{}", ran.out);
    let password = lines[0]
        .strip_prefix("KAT created. Password: ")
        .and_then(|rest| rest.strip_suffix(" (shown once; it isn't kept anywhere)"))
        .unwrap();
    assert_eq!(password.len(), 14);
    assert_eq!(lines[1], "KAT is an admin.");
    // Only its hash is kept, where only the folder's owner can read it.
    let logins = data.0.join("logins.txt");
    let kept = std::fs::read_to_string(&logins).unwrap();
    assert!(kept.contains("$argon2id$") && !kept.contains(password));
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&logins).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    // With an admin, sign-up closes by itself.
    let mut program = Program::start(&data.0);
    program.wait_for("sign-up: closed");
    let mut pcs = Pcs::new();
    let a = pcs.connect_launcher(program.port, 1, "kat", Some(password));
    assert_eq!(pcs.answer(a), Ok(()));
    pcs.until(|p| p.pcs[a].view.accounts.is_some());
    assert!(pcs.pcs[a].view.accounts.as_ref().unwrap().admin);
    let b = pcs.connect(program.port, 2, "Gamer");
    assert_eq!(pcs.answer(b), Err(live::NO_ACCOUNT.into()));
    program.stop();
    // And on its own again.
    let ran = h2live(&data.0, &["user", "list"], "");
    assert!(ran.out.contains("1/5"), "{}", ran.out);
}

#[test]
fn the_command_line_removes_makes_admins_and_gives_new_passwords() {
    let data = TempDir::new("cli-owner");
    for gamertag in ["Jun", "Kat"] {
        let ran = h2live(
            &data.0,
            &["user", "add", gamertag, "--password-stdin"],
            &format!("{PASSWORD}\n"),
        );
        assert_eq!(ran.code, 0, "{}", ran.err);
    }
    // A gamertag someone has: refused, and no password is shown.
    let ran = h2live(&data.0, &["user", "add", "jun"], "");
    assert_eq!(ran.code, 1, "{}", ran.err);
    assert!(ran.err.contains(live::GAMERTAG_TAKEN), "{}", ran.err);
    assert_eq!(ran.out, "");
    let listed = |gamertag: &str| {
        let ran = h2live(&data.0, &["user", "list"], "");
        assert_eq!(ran.code, 0, "{}", ran.err);
        let row = ran
            .out
            .lines()
            .find(|l| l.starts_with(&format!("{gamertag} ")));
        row.map(|r| r.split_whitespace().nth(1) == Some("yes"))
    };

    // Without --yes, a removal only says what it would lose.
    let ran = h2live(&data.0, &["user", "remove", "kat"], "");
    assert_eq!(ran.code, 1, "{}", ran.err);
    assert!(
        ran.out.contains("Removing KAT loses it for good"),
        "{}",
        ran.out
    );
    assert!(
        ran.err.contains("not removed: run it again with --yes"),
        "{}",
        ran.err
    );
    assert_eq!(listed("KAT"), Some(false));
    let ran = h2live(&data.0, &["user", "remove", "kat", "--yes"], "");
    assert_eq!(ran.code, 0, "{}", ran.err);
    assert!(ran.out.contains("REMOVED KAT"), "{}", ran.out);
    assert_eq!(listed("KAT"), None);

    // An admin.
    let ran = h2live(&data.0, &["user", "admin", "jun", "on"], "");
    assert_eq!(ran.code, 0, "{}", ran.err);
    assert!(ran.out.contains("JUN IS AN ADMIN NOW."), "{}", ran.out);
    assert_eq!(listed("JUN"), Some(true));

    // A new password, made up and shown once.
    let ran = h2live(&data.0, &["user", "passwd", "jun"], "");
    assert_eq!(ran.code, 0, "{}", ran.err);
    let password = ran
        .out
        .lines()
        .find_map(|l| l.strip_prefix("Password: "))
        .and_then(|rest| rest.strip_suffix(" (shown once; it isn't kept anywhere)"))
        .unwrap()
        .to_string();
    assert_eq!(password.len(), 14);
    let kept = std::fs::read_to_string(data.0.join("logins.txt")).unwrap();
    assert!(!kept.contains(&password));

    // It signs in from a new PC; the old one doesn't.
    let mut program = Program::start(&data.0);
    let mut pcs = Pcs::new();
    let a = pcs.connect_launcher(program.port, 1, "jun", Some(&password));
    assert_eq!(pcs.answer(a), Ok(()));
    let b = pcs.connect_launcher(program.port, 2, "jun", Some(PASSWORD));
    assert_eq!(pcs.answer(b), Err(live::WRONG_PASSWORD.into()));

    // Renamed (to a name that keeps the rules), while the program runs.
    let ran = h2live(&data.0, &["user", "rename", "jun", "1bad"], "");
    assert_eq!(ran.code, 2, "{}", ran.err);
    let ran = h2live(&data.0, &["user", "rename", "jun", "Jun Two"], "");
    let renamed = (ran.code, ran.out.as_str());
    assert_eq!(renamed, (0, "JUN IS JUN TWO NOW\n"), "{}", ran.err);
    assert!(listed("JUN TWO").is_some());

    // Signed out of all PCs: the one it was on needs the password again.
    let ran = h2live(&data.0, &["user", "revoke", "jun two"], "");
    let revoked = (ran.code, ran.out.as_str());
    assert_eq!(
        revoked,
        (0, "JUN TWO IS SIGNED OUT OF ALL PCS.\n"),
        "{}",
        ran.err
    );
    let c = pcs.connect_launcher(program.port, 1, "Jun Two", None);
    assert_eq!(pcs.answer(c), Err(live::PASSWORD_NEEDED.into()));
    program.stop();
    said_nothing_secret(&program, &[PASSWORD, &password]);
}

#[test]
fn a_gamertag_starting_with_a_dash_goes_after_double_dash() {
    let data = TempDir::new("cli-dash");
    // An account a PC made by signing in (as on an older server) may start
    // with a dash.
    let mut program = Program::start(&data.0);
    program.wait_for("sign-up: open");
    let mut pcs = Pcs::new();
    let a = pcs.connect(program.port, 1, "-Kat-");
    assert_eq!(pcs.answer(a), Ok(()));
    assert_eq!(pcs.pcs[a].view.welcome.as_ref().unwrap().gamertag, "-KAT-");
    // Without "--", it's taken for a flag; after it, it's the gamertag.
    let ran = h2live(&data.0, &["user", "admin", "-KAT-", "on"], "");
    assert_eq!(ran.code, 2, "{}", ran.err);
    assert!(ran.err.contains("what's -KAT-?"), "{}", ran.err);
    let ran = h2live(&data.0, &["user", "admin", "--", "-kat-", "on"], "");
    assert_eq!(ran.code, 0, "{}", ran.err);
    assert!(ran.out.starts_with("-KAT- IS AN ADMIN NOW."), "{}", ran.out);
    let ran = h2live(
        &data.0,
        &["user", "passwd", "--password-stdin", "--", "-KAT-"],
        &format!("{PASSWORD}\n"),
    );
    assert_eq!(ran.code, 0, "{}", ran.err);
    assert!(
        ran.out.starts_with("-KAT- HAS A NEW PASSWORD."),
        "{}",
        ran.out
    );
    program.stop();
    let ran = h2live(&data.0, &["user", "list"], "");
    let row = ran.out.lines().find(|l| l.starts_with("-KAT- ")).unwrap();
    let columns: Vec<&str> = row.split_whitespace().collect();
    assert_eq!(columns[1..4], ["yes", "yes", "1/5"], "{}", ran.out);
}

#[test]
fn a_password_argument_is_a_usage_error() {
    let data = TempDir::new("cli-usage");
    for args in [
        &["user", "add", "Kat", "hunter2"][..],
        &["user", "passwd", "Kat", "hunter2"],
        &["user", "add", "Kat", "--password", "hunter2"],
        &["user", "add", "1Kat"],
        &["user", "remove"],
        &["user", "list", "--yes"],
        &["useradd", "Kat"],
    ] {
        let ran = h2live(&data.0, args, "");
        assert_eq!(ran.code, 2, "{args:?}: {}", ran.err);
        assert!(ran.err.contains("usage: h2live user list"), "{}", ran.err);
        assert!(!ran.err.contains("hunter2"), "{}", ran.err);
        assert_eq!(ran.out, "");
    }
    assert!(!data.0.join("accounts.txt").exists());
    assert!(!data.0.join("logins.txt").exists());
}

#[test]
fn the_command_line_opens_only_a_folder_it_owns() {
    // Files it made in someone else's folder (root's, run without the
    // h2live wrapper on the server) would be theirs, and the program
    // couldn't open them.
    let data = TempDir::new("cli-not-owned");
    let folder = if rustix::process::geteuid().is_root() {
        // As root: a folder that is nobody's.
        std::os::unix::fs::chown(&data.0, Some(65534), Some(65534)).unwrap();
        data.0.clone()
    } else {
        // Otherwise: root's (and never written to, even by a broken build).
        PathBuf::from("/")
    };
    for (args, stdin) in [
        (&["user", "list"][..], ""),
        (
            &["user", "add", "Kat", "--password-stdin"][..],
            "K7MP-4QR9-XW2T\n",
        ),
    ] {
        let ran = h2live(&folder, args, stdin);
        assert_eq!(ran.code, 1, "{args:?}: {}", ran.err);
        let why = "run this as the folder's owner";
        assert!(ran.err.contains(why), "{args:?}: {}", ran.err);
        for file in ["h2live.lock", "logins.txt", "accounts.txt"] {
            assert!(!folder.join(file).exists(), "{file}");
        }
    }
}

#[test]
fn the_command_line_refuses_a_missing_folder() {
    let data = TempDir::new("cli-missing");
    let missing = data.0.join("not-here");
    let ran = h2live(&missing, &["user", "list"], "");
    assert_eq!(ran.code, 1);
    let why = format!(
        "no h2live data in {}; pass --data or set H2LIVE_DATA",
        missing.display()
    );
    assert!(ran.err.contains(&why), "{}", ran.err);
    let elsewhere = missing.join("deeper");
    let ran = h2live(
        &data.0,
        &["user", "list", "--data", elsewhere.to_str().unwrap()],
        "",
    );
    assert_eq!(ran.code, 1);
    assert!(!missing.exists());
}
