//! The server's command line: `h2live user ...` makes and changes the
//! accounts on a server whose owner makes them (see `USAGE`).
//!
//! It finds the data folder as the server does (H2LIVE_DATA, h2live.txt's
//! `data=`, or `h2live-data`), or `--data DIR`, and never makes one. If the
//! server is running on that folder (it holds `h2live.lock`), the request
//! goes to it over its admin socket and it answers; otherwise the command
//! line opens the folder itself, holding the lock meanwhile so the server
//! can't start on it until it's done. Either way `h2live::admin` answers.
//!
//! Passwords are made up here (or read from the standard input), hashed
//! here, and only their hashes go to the server. A made-up password is
//! shown once and isn't kept anywhere. A password is never an argument (it
//! would stay in the shell's history and show in the process list): any
//! extra word is refused.

use crate::{lock, settings};
use h2live::admin::{self, Request};
use h2live::password::{self, Cost, Secret};
use h2live::server::{date, log_to_stderr, Server};
use std::path::{Path, PathBuf};
use std::time::Instant;
use zeroize::Zeroizing;

/// (The install script's `h2live` wrapper looks for its first line,
/// "usage: h2live user list", to tell a program with this command line
/// from an older one: keep it.)
const USAGE: &str = "\
usage: h2live user list
       h2live user add GAMERTAG [--admin] [--password-stdin]
       h2live user passwd GAMERTAG [--password-stdin]
       h2live user revoke GAMERTAG
       h2live user remove GAMERTAG --yes
       h2live user admin GAMERTAG on|off
       h2live user rename GAMERTAG NEW
       h2live user key
Any of them takes --data DIR, the server's data folder (else H2LIVE_DATA,
h2live.txt's data=, or h2live-data). A gamertag with a space goes in quotes;
one starting with - goes after --, which ends the flags.

  list      every account, with its PCs
  add       a new account; its password is made up and shown once (or read
            from one line of the standard input with --password-stdin);
            --admin makes it an admin, who can make accounts from the
            launcher's ACCOUNTS screen
  passwd    a new password for an account (made up, or --password-stdin);
            the PCs it's on stay signed in
  revoke    sign an account out of every PC (it signs in again with its
            password)
  remove    remove an account (without --yes, says what would be lost); for
            good while sign-up is closed
  admin     make an account an admin, or not
  rename    give an account another gamertag
  key       the server's identity key, which launchers check";

/// What went wrong: the arguments (exit code 2) or the request (1).
enum Failure {
    Usage(String),
    Error(String),
}

impl From<String> for Failure {
    fn from(why: String) -> Failure {
        Failure::Error(why)
    }
}

/// Run the command line on `args` (those after the program's name). The
/// exit code: 0 done, 1 refused or failed, 2 not understood.
pub fn main(args: Vec<String>) -> i32 {
    // The answer goes to the standard output; what's logged to the error.
    log_to_stderr();
    if matches!(args[..], [ref a] if ["help", "--help", "-h"].contains(&a.as_str())) {
        println!("{USAGE}");
        return 0;
    }
    match run(args) {
        Ok(()) => 0,
        Err(Failure::Usage(why)) => {
            eprintln!("h2live: {why}\n{USAGE}");
            2
        }
        Err(Failure::Error(why)) => {
            eprintln!("h2live: {why}");
            1
        }
    }
}

/// The arguments, sorted out.
struct Args {
    words: Vec<String>,
    data: Option<PathBuf>,
    admin: bool,
    stdin: bool,
    yes: bool,
}

fn parse(args: Vec<String>) -> Result<Args, Failure> {
    let mut parsed = Args {
        words: Vec::new(),
        data: None,
        admin: false,
        stdin: false,
        yes: false,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data" => {
                let dir = args.next().ok_or(usage("--data needs a folder"))?;
                parsed.data = Some(dir.into());
            }
            "--admin" => parsed.admin = true,
            "--password-stdin" => parsed.stdin = true,
            "--yes" => parsed.yes = true,
            // The rest are words, even those starting with a dash (an older
            // server's gamertag may).
            "--" => {
                parsed.words.extend(args.by_ref());
                break;
            }
            _ if arg.starts_with("--data=") => parsed.data = Some(arg[7..].into()),
            _ if arg.starts_with('-') => return Err(usage(&format!("what's {arg}?"))),
            _ => parsed.words.push(arg),
        }
    }
    Ok(parsed)
}

fn usage(why: &str) -> Failure {
    Failure::Usage(why.to_string())
}

/// What `user` was asked to do.
enum Ask {
    List,
    Add { gamertag: String },
    Passwd { gamertag: String },
    Revoke { gamertag: String },
    Remove { gamertag: String },
    Admin { gamertag: String, on: bool },
    Rename { gamertag: String, new: String },
    Key,
}

fn run(args: Vec<String>) -> Result<(), Failure> {
    let args = parse(args)?;
    let words: Vec<&str> = args.words.iter().map(String::as_str).collect();
    let tag = |t: &str| t.to_string();
    let ask = match words[..] {
        ["user", "list"] => Ask::List,
        ["user", "add", gamertag] => Ask::Add {
            gamertag: tag(gamertag),
        },
        ["user", "passwd", gamertag] => Ask::Passwd {
            gamertag: tag(gamertag),
        },
        ["user", "revoke", gamertag] => Ask::Revoke {
            gamertag: tag(gamertag),
        },
        ["user", "remove", gamertag] => Ask::Remove {
            gamertag: tag(gamertag),
        },
        ["user", "admin", gamertag, on @ ("on" | "off")] => Ask::Admin {
            gamertag: tag(gamertag),
            on: on == "on",
        },
        ["user", "rename", gamertag, new] => Ask::Rename {
            gamertag: tag(gamertag),
            new: tag(new),
        },
        ["user", "key"] => Ask::Key,
        ["user", "add" | "passwd", _, _, ..] => {
            return Err(usage(
                "too many words (a password is never an argument: it's made up, or read with --password-stdin)",
            ));
        }
        ["user", ..] => return Err(usage("that isn't a user command")),
        _ => return Err(usage("the only command is \"user\"")),
    };
    // Each flag only where it means something.
    let adds = matches!(ask, Ask::Add { .. });
    let passwords = adds || matches!(ask, Ask::Passwd { .. });
    if args.admin && !adds {
        return Err(usage("--admin is only for add"));
    }
    if args.stdin && !passwords {
        return Err(usage("--password-stdin is only for add and passwd"));
    }
    if args.yes && !matches!(ask, Ask::Remove { .. }) {
        return Err(usage("--yes is only for remove"));
    }
    // Gamertags the owner gives keep the rules.
    let named = |gamertag: &str| password::check_gamertag(gamertag).map_err(usage);
    let ask = match ask {
        Ask::Add { gamertag } => Ask::Add {
            gamertag: named(&gamertag)?,
        },
        Ask::Rename { gamertag, new } => Ask::Rename {
            gamertag,
            new: named(&new)?,
        },
        ask => ask,
    };
    let mut settings = settings().map_err(Failure::Error)?;
    if let Some(data) = args.data {
        settings.data = data;
    }
    let data = settings.data.clone();
    if !data.is_dir() {
        return Err(Failure::Error(format!(
            "no h2live data in {}; pass --data or set H2LIVE_DATA",
            data.display()
        )));
    }
    let folder = Folder {
        data,
        secret: settings.secret.clone(),
        signup: settings.signup,
    };
    match ask {
        Ask::List => list(&folder),
        Ask::Add { gamertag } => {
            let (phc, made_up) = new_password(args.stdin)?;
            let add = Request::Add {
                gamertag: gamertag.clone(),
                phc,
                admin: args.admin,
            };
            done(folder.ask(add)?)?;
            match made_up {
                Some(p) => println!(
                    "{gamertag} created. Password: {} (shown once; it isn't kept anywhere)",
                    p.as_str()
                ),
                None => println!("{gamertag} created."),
            }
            if args.admin {
                println!("{gamertag} is an admin.");
            }
            Ok(())
        }
        Ask::Passwd { gamertag } => {
            let (phc, made_up) = new_password(args.stdin)?;
            let text = done(folder.ask(Request::Passwd { gamertag, phc })?)?;
            println!("{text}");
            if let Some(p) = made_up {
                println!(
                    "Password: {} (shown once; it isn't kept anywhere)",
                    p.as_str()
                );
            }
            Ok(())
        }
        Ask::Revoke { gamertag } => say(folder.ask(Request::Revoke { gamertag })?),
        Ask::Remove { gamertag } if args.yes => say(folder.ask(Request::Remove { gamertag })?),
        Ask::Remove { gamertag } => would_lose(&folder, &gamertag),
        Ask::Admin { gamertag, on } => say(folder.ask(Request::Admin { gamertag, on })?),
        Ask::Rename { gamertag, new } => say(folder.ask(Request::Rename { gamertag, new })?),
        Ask::Key => {
            let answer = done(folder.ask(Request::Key)?)?;
            let (key, short) = answer.split_once('\t').unwrap_or((&answer, ""));
            println!("identity key: {key}");
            println!("short code:   {short}");
            Ok(())
        }
    }
}

/// A new password's hash, and the password if it was made up (to show):
/// made up, or (`stdin`) one line of the standard input.
fn new_password(stdin: bool) -> Result<(String, Option<Secret>), Failure> {
    let (typed, made_up) = if stdin {
        let mut line = Zeroizing::new(String::with_capacity(256));
        std::io::stdin()
            .read_line(&mut line)
            .map_err(|e| format!("can't read the password: {e}"))?;
        let typed = line.trim_end_matches(['\r', '\n']);
        password::check(typed).map_err(String::from)?;
        (Secret::new(typed), None)
    } else {
        let p = password::generate();
        (p.clone(), Some(p))
    };
    let start = Instant::now();
    let phc = password::hash(typed.as_str(), Cost::Real).map_err(String::from)?;
    eprintln!("hashing took {} ms", start.elapsed().as_millis());
    Ok((phc, made_up))
}

/// The answer's text, if it's `ok`; else the error.
fn done(answer: Vec<String>) -> Result<String, Failure> {
    let last = answer.last().map_or("", String::as_str);
    match last.split_once('\t') {
        Some(("ok", text)) => Ok(text.to_string()),
        Some(("error", text)) => Err(Failure::Error(text.to_string())),
        _ => Err(Failure::Error(format!("h2live answered {last:?}"))),
    }
}

/// Print the answer's text, if it's `ok`.
fn say(answer: Vec<String>) -> Result<(), Failure> {
    println!("{}", done(answer)?);
    Ok(())
}

/// An account as `list` answers it.
struct Row {
    id: String,
    gamertag: String,
    admin: bool,
    password: bool,
    pcs: Vec<(u64, String)>,
    online: bool,
    level: String,
    created: u64,
}

/// The accounts in `list`'s answer.
fn rows(answer: &[String]) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    for line in answer {
        let words: Vec<&str> = line.split('\t').collect();
        match words[..] {
            ["row", id, gamertag, admin, password, _, online, level, created] => rows.push(Row {
                id: id.to_string(),
                gamertag: gamertag.to_string(),
                admin: admin == "1",
                password: password == "1",
                pcs: Vec::new(),
                online: online == "1",
                level: level.to_string(),
                created: created.parse().unwrap_or(0),
            }),
            ["pc", id, added, name] => {
                if let Some(row) = rows.iter_mut().rev().find(|r| r.id == id) {
                    row.pcs.push((added.parse().unwrap_or(0), name.to_string()));
                }
            }
            _ => {}
        }
    }
    rows
}

fn yes(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

/// Print every account, with its PCs.
fn list(folder: &Folder) -> Result<(), Failure> {
    let answer = folder.ask(Request::List)?;
    let total = done(answer.clone())?;
    let rows = rows(&answer);
    if !rows.is_empty() {
        println!(
            "{:<15}  {:<5}  {:<8}  {:<4}  {:<6}  {:<5}  CREATED",
            "GAMERTAG", "ADMIN", "PASSWORD", "PCS", "ONLINE", "LEVEL"
        );
    }
    for r in &rows {
        println!(
            "{:<15}  {:<5}  {:<8}  {:<4}  {:<6}  {:<5}  {}",
            r.gamertag,
            yes(r.admin),
            yes(r.password),
            format!("{}/{}", r.pcs.len(), h2live::logins::MAX_PCS),
            yes(r.online),
            r.level,
            date(r.created)
        );
        for (added, name) in &r.pcs {
            println!("{:<15}  PC \"{name}\", added {}", "", date(*added));
        }
    }
    println!("{}", total.to_lowercase());
    Ok(())
}

/// Say what removing `gamertag` would lose, and that --yes does it. (An
/// error, so it isn't taken for done.)
fn would_lose(folder: &Folder, gamertag: &str) -> Result<(), Failure> {
    let answer = folder.ask(Request::List)?;
    done(answer.clone())?;
    let rows = rows(&answer);
    let wanted = gamertag.trim();
    let Some(r) = rows
        .iter()
        .find(|r| r.gamertag.eq_ignore_ascii_case(wanted))
    else {
        return Err(Failure::Error(format!(
            "NO ACCOUNT IS CALLED {}",
            wanted.to_uppercase()
        )));
    };
    let names: Vec<&str> = r.pcs.iter().map(|(_, n)| n.as_str()).collect();
    let pcs = match names.len() {
        0 => "none".to_string(),
        n => format!("{n}: {}", names.join(", ")),
    };
    println!(
        "Removing {} loses it for good: its level ({}), its stats, its friends list \
         and the PCs it's on ({pcs}).",
        r.gamertag, r.level
    );
    // (An account a PC's key named, by signing up or on an older server, is
    // that key's again while sign-up is open: see `Server::prove`.)
    println!(
        "(While sign-up is open, an account a PC made by signing in, here or on an \
         older server, comes back when that PC signs in again, its stat card bringing \
         back its levels.)"
    );
    if r.online {
        println!("{} is online now, and would be signed out.", r.gamertag);
    }
    Err(Failure::Error(
        "not removed: run it again with --yes to remove it".into(),
    ))
}

/// The data folder, and how to open it when the server isn't running.
struct Folder {
    data: PathBuf,
    secret: Option<String>,
    signup: h2live::server::SignUp,
}

impl Folder {
    /// The answer to `request`: from the server, if it's running on the
    /// folder, or else from the folder itself.
    fn ask(&self, request: Request) -> Result<Vec<String>, String> {
        let shown = |e: std::io::Error| format!("{}: {e}", self.data.display());
        match lock(&self.data, false) {
            Ok(Some(held)) => self.offline(held, request),
            Ok(None) => online(&self.data, request),
            // No server has run on it since there were locks.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                owned(&self.data)?;
                match lock(&self.data, true).map_err(shown)? {
                    Some(held) => self.offline(held, request),
                    None => online(&self.data, request),
                }
            }
            Err(e) => Err(shown(e)),
        }
    }

    /// Answer `request` from the folder, while holding its lock (`_held`).
    fn offline(&self, _held: std::fs::File, request: Request) -> Result<Vec<String>, String> {
        owned(&self.data)?;
        let mut server = Server::open(&self.data, self.secret.as_deref())?;
        server.set_sign_up(self.signup);
        Ok(admin::answer(&mut server, request, 0.0))
    }
}

/// The folder is ours (so files made in it stay its owner's): on Unix, we
/// run as its owner.
#[cfg(unix)]
fn owned(folder: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let owner = std::fs::metadata(folder)
        .map_err(|e| format!("{}: {e}", folder.display()))?
        .uid();
    if owner != rustix::process::geteuid().as_raw() {
        return Err("run this as the folder's owner (on the server: the h2live command)".into());
    }
    Ok(())
}

#[cfg(not(unix))]
fn owned(_: &Path) -> Result<(), String> {
    Ok(())
}

/// Ask the server running on `folder`, over its admin socket.
#[cfg(unix)]
fn online(folder: &Path, request: Request) -> Result<Vec<String>, String> {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    let path = folder.join("admin.sock");
    let mut stream = UnixStream::connect(&path).map_err(|e| {
        format!(
            "h2live is running, but its admin socket {} can't be reached: {e}",
            path.display()
        )
    })?;
    let wait = Some(std::time::Duration::from_secs(20));
    stream.set_read_timeout(wait).map_err(|e| e.to_string())?;
    stream.set_write_timeout(wait).map_err(|e| e.to_string())?;
    let line = format!("{}\n", request.line());
    stream
        .write_all(line.as_bytes())
        .map_err(|e| format!("can't ask h2live: {e}"))?;
    let mut answer = String::new();
    stream
        .read_to_string(&mut answer)
        .map_err(|e| format!("h2live didn't answer: {e}"))?;
    if answer.trim().is_empty() {
        return Err("h2live didn't answer".into());
    }
    Ok(answer.lines().map(String::from).collect())
}

#[cfg(not(unix))]
fn online(_: &Path, _: Request) -> Result<Vec<String>, String> {
    Err("h2live is running; stop it and try again".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `parse`'s words, or why it refused.
    fn words(args: &[&str]) -> Result<Vec<String>, String> {
        match parse(args.iter().map(|a| a.to_string()).collect()) {
            Ok(parsed) => Ok(parsed.words),
            Err(Failure::Usage(why) | Failure::Error(why)) => Err(why),
        }
    }

    #[test]
    fn a_gamertag_starting_with_a_dash_goes_after_double_dash() {
        // Taken for a flag it doesn't know...
        assert_eq!(
            words(&["user", "admin", "-KAT-", "on"]),
            Err("what's -KAT-?".to_string())
        );
        // ...but after "--", every argument is a word.
        assert_eq!(
            words(&["user", "admin", "--", "-KAT-", "on"]).unwrap(),
            ["user", "admin", "-KAT-", "on"]
        );
        // Flags before it still count.
        let args = ["user", "passwd", "--password-stdin", "--", "--NINJA--"];
        let parsed = parse(args.iter().map(|a| a.to_string()).collect())
            .ok()
            .unwrap();
        assert!(parsed.stdin);
        assert_eq!(parsed.words, ["user", "passwd", "--NINJA--"]);
        // One after it is a word, so it's never taken as a flag (and a
        // password as a word is still refused).
        assert_eq!(
            words(&["user", "add", "--", "KAT", "--admin"]).unwrap(),
            ["user", "add", "KAT", "--admin"]
        );
        assert!(matches!(
            run(vec![
                "user".into(),
                "add".into(),
                "--".into(),
                "KAT".into(),
                "--admin".into()
            ]),
            Err(Failure::Usage(_))
        ));
    }
}
