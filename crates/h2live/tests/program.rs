//! The h2live program itself, as players' PCs find it: over TCP on this
//! PC, with web requests and WebSockets on the one port. (Unix only: it's
//! stopped as Ctrl+C would, with a signal.)

#![cfg(unix)]

use ed25519_dalek::SigningKey;
use h2live::client::{LiveClient, LiveEvent, Profile};
use h2net::live::{self, ToServer};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
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
        let (mut program, stdout) = Program::launch(data);
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
        let mut child = Command::new(env!("CARGO_BIN_EXE_h2live"))
            .env("PORT", "0")
            .env("H2LIVE_DATA", data)
            .env_remove("H2LIVE_SECRET")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
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
