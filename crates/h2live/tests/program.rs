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
use std::process::{Child, Command, Stdio};
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
        let mut child = Command::new(env!("CARGO_BIN_EXE_h2live"))
            .env("PORT", "0")
            .env("H2LIVE_DATA", data)
            .env_remove("H2LIVE_SECRET")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        });
        let mut program = Program {
            child,
            lines,
            said: Vec::new(),
            port: 0,
        };
        let line = program.wait_for("listening on port ");
        let after = line.split("listening on port ").nth(1).unwrap();
        program.port = after.split(' ').next().unwrap().parse().unwrap();
        program
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
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        write!(s, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        let mut answer = String::new();
        s.read_to_string(&mut answer).unwrap();
        answer
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
