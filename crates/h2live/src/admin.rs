//! What the server's command line (`h2live user ...`) asks the server, and
//! how it's answered.
//!
//! The command line runs as its own process. While the server runs, it
//! sends one request over the server's admin socket (a Unix socket in the
//! data folder that only the folder's owner can open) and prints the
//! answer; while it's stopped, it opens the data folder itself and asks a
//! `Server` there. Either way the request and the answer are the lines
//! below, so both take the same path through `answer`.
//!
//! A request is one line of words separated by tabs:
//!
//! ```text
//! list
//! add GAMERTAG PHC 0|1
//! passwd GAMERTAG PHC
//! revoke GAMERTAG
//! remove GAMERTAG
//! admin GAMERTAG on|off
//! rename GAMERTAG NEW
//! key
//! ```
//!
//! Passwords are hashed by the command line, so only their hashes (PHC
//! strings) ever reach the server; neither is ever logged. The answer is
//! lines of tab-separated words too, the last always `ok TEXT` or
//! `error TEXT`:
//!
//! ```text
//! row ID GAMERTAG ADMIN0|1 PASSWORD0|1 PCS ONLINE0|1 BEST CREATED
//! pc ID ADDED NAME
//! ok IDENTITY-KEY-HEX SHORT-CODE      (for key)
//! ```

use crate::password;
use crate::seal;
use crate::server::{Actor, Command, Server};
use crate::store::hex;
use h2net::live::{ROW_ADMIN, ROW_ONLINE, ROW_PASSWORD};

/// One request from the command line. (Its Debug doesn't show hashes.)
#[derive(Clone, PartialEq, Eq)]
pub enum Request {
    /// Every account, with its PCs.
    List,
    /// A new account with a password (its hash).
    Add {
        gamertag: String,
        phc: String,
        admin: bool,
    },
    /// A new password; the account's PCs stay signed in.
    Passwd {
        gamertag: String,
        phc: String,
    },
    /// Sign the account out of every PC.
    Revoke {
        gamertag: String,
    },
    /// Remove the account for good.
    Remove {
        gamertag: String,
    },
    /// Make the account an admin, or not.
    Admin {
        gamertag: String,
        on: bool,
    },
    Rename {
        gamertag: String,
        new: String,
    },
    /// The server's identity key, which launchers pin.
    Key,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Request::List => write!(f, "List"),
            Request::Add {
                gamertag, admin, ..
            } => write!(f, "Add {{ gamertag: {gamertag:?}, admin: {admin} }}"),
            Request::Passwd { gamertag, .. } => write!(f, "Passwd {{ gamertag: {gamertag:?} }}"),
            Request::Revoke { gamertag } => write!(f, "Revoke {{ gamertag: {gamertag:?} }}"),
            Request::Remove { gamertag } => write!(f, "Remove {{ gamertag: {gamertag:?} }}"),
            Request::Admin { gamertag, on } => {
                write!(f, "Admin {{ gamertag: {gamertag:?}, on: {on} }}")
            }
            Request::Rename { gamertag, new } => {
                write!(f, "Rename {{ gamertag: {gamertag:?}, new: {new:?} }}")
            }
            Request::Key => write!(f, "Key"),
        }
    }
}

impl Request {
    /// The request as one line (with no line break).
    pub fn line(&self) -> String {
        let words: Vec<&str> = match self {
            Request::List => vec!["list"],
            Request::Add {
                gamertag,
                phc,
                admin,
            } => vec!["add", gamertag, phc, if *admin { "1" } else { "0" }],
            Request::Passwd { gamertag, phc } => vec!["passwd", gamertag, phc],
            Request::Revoke { gamertag } => vec!["revoke", gamertag],
            Request::Remove { gamertag } => vec!["remove", gamertag],
            Request::Admin { gamertag, on } => {
                vec!["admin", gamertag, if *on { "on" } else { "off" }]
            }
            Request::Rename { gamertag, new } => vec!["rename", gamertag, new],
            Request::Key => vec!["key"],
        };
        words.join("\t")
    }

    /// The request in `line` (a line break at its end is fine). None if it
    /// isn't one.
    pub fn parse(line: &str) -> Option<Request> {
        let line = line.strip_suffix('\n').unwrap_or(line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        let words: Vec<&str> = line.split('\t').collect();
        let s = |w: &str| w.to_string();
        let request = match words[..] {
            ["list"] => Request::List,
            ["add", gamertag, phc, admin] => Request::Add {
                gamertag: s(gamertag),
                phc: s(phc),
                admin: match admin {
                    "0" => false,
                    "1" => true,
                    _ => return None,
                },
            },
            ["passwd", gamertag, phc] => Request::Passwd {
                gamertag: s(gamertag),
                phc: s(phc),
            },
            ["revoke", gamertag] => Request::Revoke {
                gamertag: s(gamertag),
            },
            ["remove", gamertag] => Request::Remove {
                gamertag: s(gamertag),
            },
            ["admin", gamertag, on] => Request::Admin {
                gamertag: s(gamertag),
                on: match on {
                    "on" => true,
                    "off" => false,
                    _ => return None,
                },
            },
            ["rename", gamertag, new] => Request::Rename {
                gamertag: s(gamertag),
                new: s(new),
            },
            ["key"] => Request::Key,
            _ => return None,
        };
        Some(request)
    }
}

/// Answer `request` from the command line, at the server's time `now`:
/// the lines to send back, the last `ok TEXT` or `error TEXT`.
pub fn answer(server: &mut Server, request: Request, now: f64) -> Vec<String> {
    let mut lines = Vec::new();
    let find = |gamertag: &str| {
        server.find(gamertag).ok_or_else(|| {
            let name = gamertag.trim().to_uppercase();
            format!("NO ACCOUNT IS CALLED {}", shown(&name))
        })
    };
    let result = match request {
        Request::List => {
            for row in server.account_rows() {
                let created = server.account(row.account).map_or(0, |a| a.created);
                let flag = |f: u8| if row.flags & f != 0 { "1" } else { "0" };
                lines.push(format!(
                    "row\t{:016x}\t{}\t{}\t{}\t{}\t{}\t{}\t{created}",
                    row.account,
                    row.gamertag,
                    flag(ROW_ADMIN),
                    flag(ROW_PASSWORD),
                    row.pcs,
                    flag(ROW_ONLINE),
                    row.best,
                ));
                for pc in server.devices_of(row.account) {
                    let name = shown(&pc.name);
                    lines.push(format!("pc\t{:016x}\t{}\t{name}", row.account, pc.added));
                }
            }
            let n = lines.iter().filter(|l| l.starts_with("row\t")).count();
            Ok(if n == 1 {
                "1 ACCOUNT".to_string()
            } else {
                format!("{n} ACCOUNTS")
            })
        }
        Request::Add {
            gamertag,
            phc,
            admin,
        } => password::check_gamertag(&gamertag)
            .map_err(String::from)
            .and_then(|gamertag| {
                let add = Command::Add {
                    gamertag,
                    phc,
                    admin,
                };
                server.command(Actor::Console, add, now)
            }),
        Request::Passwd { gamertag, phc } => find(&gamertag).and_then(|account| {
            let set = Command::SetPassword { account, phc };
            server.command(Actor::Console, set, now)
        }),
        Request::Revoke { gamertag } => find(&gamertag)
            .and_then(|account| server.command(Actor::Console, Command::SignOut { account }, now)),
        Request::Remove { gamertag } => find(&gamertag)
            .and_then(|account| server.command(Actor::Console, Command::Remove { account }, now)),
        Request::Admin { gamertag, on } => find(&gamertag).and_then(|account| {
            let set = Command::SetAdmin { account, admin: on };
            server.command(Actor::Console, set, now)
        }),
        Request::Rename { gamertag, new } => find(&gamertag).and_then(|account| {
            let rename = Command::Rename {
                account,
                gamertag: new,
            };
            server.command(Actor::Console, rename, now)
        }),
        Request::Key => {
            let key = server.identity_key();
            return vec![format!("ok\t{}\t{}", hex(&key), seal::short_code(&key))];
        }
    };
    lines.push(match result {
        Ok(text) => format!("ok\t{}", shown(&text)),
        Err(text) => format!("error\t{}", shown(&text)),
    });
    lines
}

/// `text` as one tab-separated word can show it: tabs and line breaks (and
/// anything else that isn't printable) as spaces.
fn shown(text: &str) -> String {
    let word = |c: char| if c.is_control() { ' ' } else { c };
    text.chars().map(word).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::password::Cost;

    fn every_request() -> Vec<Request> {
        let phc = password::hash("K7MP-4QR9-XW2T", Cost::Cheap).unwrap();
        let tag = |t: &str| t.to_string();
        vec![
            Request::List,
            Request::Add {
                gamertag: tag("KAT"),
                phc: phc.clone(),
                admin: true,
            },
            Request::Add {
                gamertag: tag("TEST C"),
                phc: phc.clone(),
                admin: false,
            },
            Request::Passwd {
                gamertag: tag("KAT"),
                phc,
            },
            Request::Revoke {
                gamertag: tag("KAT"),
            },
            Request::Remove {
                gamertag: tag("KAT"),
            },
            Request::Admin {
                gamertag: tag("KAT"),
                on: true,
            },
            Request::Admin {
                gamertag: tag("KAT"),
                on: false,
            },
            Request::Rename {
                gamertag: tag("KAT"),
                new: tag("JUN"),
            },
            Request::Key,
        ]
    }

    #[test]
    fn requests_read_back_as_written_and_show_no_hashes() {
        for request in every_request() {
            let line = request.line();
            assert!(!line.contains('\n'));
            assert_eq!(Request::parse(&line), Some(request.clone()), "{line}");
            assert_eq!(Request::parse(&format!("{line}\n")), Some(request.clone()));
            assert!(!format!("{request:?}").contains("$argon2id"), "{request:?}");
        }
        for bad in [
            "",
            "lst",
            "list\textra",
            "add\tKAT\tPHC",
            "add\tKAT\tPHC\t2",
            "admin\tKAT\tmaybe",
            "remove",
            "key\tnow",
            "remove KAT",
        ] {
            assert_eq!(Request::parse(bad), None, "{bad:?}");
        }
    }
}
