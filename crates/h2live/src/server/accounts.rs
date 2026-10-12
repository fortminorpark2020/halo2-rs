//! Accounts the server's owner makes, and who may sign in to each.
//!
//! Who may sign in is kept in logins.txt (`crate::logins`): for each
//! account an admin flag, a password hash or none, and up to `MAX_PCS`
//! PCs ("devices") that sign in by their key alone. accounts.txt and stat
//! cards are as they were.
//!
//! Signing in, once a PC proved it holds its key:
//!
//! - its key is a device of an account: it signs in to that one;
//! - else an account is named by its key (one whose PCs were all signed
//!   out): a launcher is asked for the password (PASSWORD_NEEDED), the game
//!   is told it was signed out (SIGNED_OUT);
//! - else, with sign-up closed: the same for a launcher, and the game is
//!   told the server is invite only (NO_ACCOUNT);
//! - else (sign-up open) it gets a new account named by its key, or one a
//!   stat card brings back, as it always has, and its key as a device.
//!
//! A launcher on a PC an account doesn't know proves the account's
//! password instead (PROVE_PASSWORD, sealed: see `crate::seal`). Right, its
//! key becomes a device of the account (taken from any other account that
//! had it). A wrong password, an account without a password and a gamertag
//! no account has get the same answer after the same work: a box that
//! opens costs one hash whatever it names (with no password to check, a
//! stand-in no one knows is checked), and 5 wrong tries lock a gamertag no
//! account has as they lock an account. So neither the answer, how long it
//! takes, nor the lock tells a guesser which gamertags are accounts. Each
//! password checked costs a hash on the server's own thread, so there are
//! limits: 30 hashes a minute for the whole server, 5 wrong passwords for
//! an account (or gamertag) or 10 from an address in 10 minutes, and 10
//! passwords checked for an account in 10 minutes, right or wrong, so
//! knowing one password can't keep the server busy (then TOO_MANY_TRIES
//! until they're older). PCs an account knows need no password, and a
//! password sent from one isn't checked, so no one can lock a player out of
//! playing by guessing.
//!
//! Sign-up is open, closed, or auto (the default): closed once any account
//! is an admin. While it's closed, or for an account with a password, no
//! LOGIN or PROFILE renames an account (only the command line does), and
//! a stat card only raises the record of an account the server has.
//!
//! Admins change accounts from their launcher (ACCOUNT, signed with the
//! PC's key over the connection's challenge and a number that only goes
//! up, so it can't be replayed or forged by someone who took over the
//! link): make an account, give it a new password, sign it out of every
//! PC, or remove it; never their own account or another admin's. Anyone
//! can change their own password, knowing the old one. The server's
//! command line (`crate::admin`) does all of that and more: it makes
//! admins and renames accounts. Every launcher signed in is sent the
//! accounts it may see (ACCOUNTS) after WELCOME, after each of its
//! requests, and whenever an account changes.
//!
//! logins.txt is written so a crash never lets anyone in: a new account's
//! sign-ins before accounts.txt has it (an account with sign-ins but no
//! account is dropped at the next start), and a removed account out of
//! accounts.txt before its sign-ins go (an account without them would be
//! adopted, its name key signing in again). Passwords, hashes and boxes
//! are never logged.

use super::{log, printable, signed_over, Server};
use crate::logins::{Device, Logins, MAX_PCS};
use crate::password::{self, Cost, Secret, Settings};
use crate::seal;
use crate::store::{self, Account};
use h2net::live::{
    self, AccountOp, AccountRow, Accounts, ClientKind, Login, Sealed, ToPc, MAX_ACCOUNT_ROWS,
    ROW_ADMIN, ROW_ONLINE, ROW_PASSWORD,
};
use h2sim::game::clean_name;
use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;
use std::path::Path;

/// Passwords checked or hashed on the server's thread in any minute, at
/// most. A hash takes 25 to 80 ms (an estimate until it's measured on the
/// owner's server), so this bounds them to a few seconds a minute.
pub const HASHES_PER_MINUTE: usize = 30;
/// Wrong passwords an account, or an address, may have in 10 minutes.
pub const WRONG_PER_ACCOUNT: usize = 5;
pub const WRONG_PER_IP: usize = 10;
/// Passwords checked to sign in to an account in 10 minutes, right or
/// wrong: each costs a hash, so whoever knows one account's password can't
/// keep the server busy for everyone else by signing in again and again.
/// (Ten is room for a player adding all five of their PCs, and some typos.)
pub const CHECKS_PER_ACCOUNT: usize = 10;
/// Passwords a player may change in 10 minutes: each costs two hashes, so
/// no one signed in can keep the server busy for everyone else.
pub const CHANGES_PER_ACCOUNT: usize = 3;
/// How long wrong passwords count (seconds).
const WRONG_WINDOW: f64 = 600.0;
/// Longest name a PC is listed under.
const MAX_PC_NAME: usize = 32;

/// What account commands are refused with.
pub const NO_SUCH_ACCOUNT: &str = "NO SUCH ACCOUNT";
pub const YOUR_OWN: &str =
    "USE CHANGE PASSWORD, OR THE SERVER'S COMMAND LINE, FOR YOUR OWN ACCOUNT.";
pub const AN_ADMINS: &str = "ONLY THE SERVER'S COMMAND LINE CHANGES AN ADMIN'S ACCOUNT.";
pub const NOT_A_HASH: &str = "THAT ISN'T A PASSWORD HASH";
pub const ONLY_THE_COMMAND_LINE: &str = "ONLY THE SERVER'S COMMAND LINE MAKES ADMINS.";
pub const RENAMES: &str = "ONLY THE SERVER'S COMMAND LINE RENAMES ACCOUNTS.";
pub const LAST_ADMIN: &str = "MAKE SOMEONE ELSE AN ADMIN FIRST.";
pub const NO_PASSWORD_YET: &str = "ASK THE SERVER'S OWNER FOR A PASSWORD FIRST.";
pub const WRONG_OLD_PASSWORD: &str = "WRONG PASSWORD";
pub const PASSWORD_CHANGED: &str = "PASSWORD CHANGED";
pub const TOO_MANY_CHANGES: &str = "YOU CHANGED YOUR PASSWORD TOO OFTEN. TRY AGAIN IN 10 MINUTES.";
pub const CANT_SAVE: &str = "THE SERVER COULDN'T SAVE ITS FILES";
/// Added to what a command did when it takes access away but logins.txt
/// couldn't be saved: it holds in memory (undoing it would let back in
/// whoever it shut out), and is asked for again.
pub const NOT_SAVED: &str =
    "BUT THE SERVER COULDN'T SAVE ITS FILES, SO IT'S UNDONE WHEN THE SERVER NEXT STARTS. TRY AGAIN.";

/// Whether PCs the server doesn't know get accounts of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SignUp {
    /// They do, as they always have.
    Open,
    /// Only the server's owner makes accounts.
    Closed,
    /// Open until any account is an admin, then closed.
    #[default]
    Auto,
}

/// Who asks for an account command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    /// The server's command line (`h2live user ...`), which may do anything.
    Console,
    /// An admin, from their launcher.
    Admin(u64),
}

/// An account command (`Server::command`). Password hashes come ready
/// made: the command line hashes in its own process, and a launcher's
/// password is hashed when its box is opened. (Its Debug doesn't show
/// them.)
#[derive(Clone, PartialEq, Eq)]
pub enum Command {
    /// A new account. Only the command line makes admins.
    Add {
        gamertag: String,
        phc: String,
        admin: bool,
    },
    /// A new password; the account's PCs stay signed in.
    SetPassword {
        account: u64,
        phc: String,
    },
    /// Sign the account out of every PC.
    SignOut {
        account: u64,
    },
    /// Remove the account for good.
    Remove {
        account: u64,
    },
    /// The command line's only.
    SetAdmin {
        account: u64,
        admin: bool,
    },
    Rename {
        account: u64,
        gamertag: String,
    },
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Command::Add {
                gamertag, admin, ..
            } => write!(f, "Add {{ gamertag: {gamertag:?}, admin: {admin} }}"),
            Command::SetPassword { account, .. } => write!(f, "SetPassword {account:016x}"),
            Command::SignOut { account } => write!(f, "SignOut {account:016x}"),
            Command::Remove { account } => write!(f, "Remove {account:016x}"),
            Command::SetAdmin { account, admin } => write!(f, "SetAdmin {account:016x} {admin}"),
            Command::Rename { account, gamertag } => {
                write!(f, "Rename {account:016x} {gamertag:?}")
            }
        }
    }
}

/// The sign-ins in `dir`'s logins.txt for `accounts`: those of accounts it
/// doesn't have dropped, and those it has without sign-ins adopted (saved
/// if anything changed and there are accounts).
pub(super) fn open_logins(dir: &Path, accounts: &BTreeMap<u64, Account>) -> Result<Logins, String> {
    let path = dir.join("logins.txt");
    let mut logins = Logins::load(&path)?.unwrap_or_default();
    let dropped = logins.keep_only(|id| accounts.contains_key(&id));
    if dropped > 0 {
        log(format_args!(
            "live: logins.txt named {} accounts.txt doesn't have: dropped",
            count(dropped, "account", "accounts")
        ));
    }
    let adopted = accounts.values().filter(|a| logins.adopt(a)).count();
    if (adopted > 0 || dropped > 0) && !accounts.is_empty() {
        logins
            .save(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    if adopted > 0 {
        log(format_args!(
            "live: {} from an older server sign in from their PCs as before",
            count(adopted, "account", "accounts")
        ));
    }
    Ok(logins)
}

/// "1 account", "2 accounts".
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// How many of `times` are within `window` seconds before `now`.
fn recent(times: &[f64], now: f64, window: f64) -> usize {
    times.iter().filter(|&&t| now - t < window).count()
}

impl Server {
    /// Whether PCs the server doesn't know get accounts (auto unless set).
    pub fn set_sign_up(&mut self, sign_up: SignUp) {
        self.sign_up = sign_up;
    }

    /// Sign-up is closed: told so, or auto with an admin.
    pub fn sign_up_closed(&self) -> bool {
        match self.sign_up {
            SignUp::Open => false,
            SignUp::Closed => true,
            SignUp::Auto => self.logins.admins() > 0,
        }
    }

    /// What the server's log says of sign-up when it starts (one or more
    /// lines).
    pub fn sign_up_line(&self) -> String {
        let admins = self.logins.admins();
        let mut lines = vec![if self.sign_up_closed() {
            "sign-up: closed (accounts are made with \"h2live user add\" or the \
             launcher's ACCOUNTS screen)"
                .to_string()
        } else if self.sign_up == SignUp::Auto {
            "sign-up: open (every new PC gets an account; it closes once there's an admin)"
                .to_string()
        } else {
            "sign-up: open (every new PC gets an account)".to_string()
        }];
        if self.sign_up_closed() && admins == 0 {
            lines.push("no admins yet: run \"h2live user admin <gamertag> on\"".into());
        }
        if self.sign_up == SignUp::Open && admins > 0 {
            lines.push(format!(
                "warning: sign-up is open (signup=open or H2LIVE_SIGNUP=open) though there {}",
                if admins == 1 {
                    "is an admin".to_string()
                } else {
                    format!("are {admins} admins")
                }
            ));
        }
        lines.join("\n")
    }

    /// How hard the password hashes made here are (`Cost::Real` unless
    /// set: tests set `Cost::Cheap`).
    pub fn set_password_cost(&mut self, cost: Cost) {
        self.cost = cost;
        self.stand_in = None;
    }

    /// A hash of a password no one has (made at random, and never kept),
    /// made with the settings most kept hashes have (this server's own when
    /// it keeps none): checked in place of a password when there's none to
    /// check, so the answer takes as long as checking a usual account's.
    /// (After a change to `Cost::Real`, the hashes made before it keep
    /// their settings, and so does the stand-in while they're most.)
    fn stand_in(&mut self) -> &str {
        let settings = self.usual_settings();
        if self.stand_in.as_ref().map(|(s, _)| *s) != Some(settings) {
            let made = password::hash_with(password::generate().as_str(), settings);
            self.stand_in = Some((settings, made.unwrap_or_default()));
        }
        self.stand_in.as_ref().map_or("", |(_, phc)| phc)
    }

    /// Make the stand-in now (as the program starts), so the first try for
    /// a gamertag no account has doesn't also wait for it to be made.
    pub fn prepare_stand_in(&mut self) {
        self.stand_in();
    }

    /// The settings most kept hashes were made with (ties go to this
    /// server's own, then to the costlier), or this server's own if it
    /// keeps none.
    fn usual_settings(&self) -> Settings {
        let own = self.cost.settings();
        let mut counts: HashMap<Settings, usize> = HashMap::new();
        for s in self.logins.hashes().filter_map(password::settings_of) {
            *counts.entry(s).or_default() += 1;
        }
        counts
            .into_iter()
            .max_by_key(|&(s, n)| (n, s == own, s))
            .map_or(own, |(s, _)| s)
    }

    /// The server's identity key (Ed25519), which launchers pin.
    pub fn identity_key(&self) -> [u8; 32] {
        self.identity.verifying_key().to_bytes()
    }

    /// The account with `gamertag`, in any case.
    pub fn find(&self, gamertag: &str) -> Option<u64> {
        let wanted = clean_name(gamertag);
        if wanted.is_empty() {
            return None;
        }
        let named = |a: &&Account| a.gamertag.eq_ignore_ascii_case(&wanted);
        self.accounts.values().find(named).map(|a| a.id)
    }

    /// Every account as ACCOUNTS lists it, by gamertag.
    pub fn account_rows(&self) -> Vec<AccountRow> {
        let mut rows: Vec<AccountRow> = self.accounts.values().map(|a| self.row(a)).collect();
        rows.sort_by(|a, b| (&a.gamertag, a.account).cmp(&(&b.gamertag, b.account)));
        rows
    }

    /// The PCs that sign in to account `id` without a password.
    pub fn devices_of(&self, id: u64) -> Vec<Device> {
        self.logins
            .get(id)
            .map_or(Vec::new(), |l| l.devices.clone())
    }

    fn row(&self, a: &Account) -> AccountRow {
        let login = self.logins.get(a.id);
        let mut flags = 0;
        if self.logins.is_admin(a.id) {
            flags |= ROW_ADMIN;
        }
        if login.is_some_and(|l| l.password.is_some()) {
            flags |= ROW_PASSWORD;
        }
        if self.pc_of(a.id).is_some() {
            flags |= ROW_ONLINE;
        }
        AccountRow {
            account: a.id,
            gamertag: a.gamertag.clone(),
            flags,
            pcs: login.map_or(0, |l| l.devices.len().min(255) as u8),
            best: a.best_level(),
        }
    }

    /// Account `id` may be renamed from LOGIN or PROFILE: sign-up is open
    /// and it has no password.
    pub(super) fn may_rename(&self, id: u64) -> bool {
        !self.sign_up_closed() && self.logins.get(id).is_none_or(|l| l.password.is_none())
    }

    /// Give account `id` sign-ins if it has none (a new account, sign-up
    /// open): its key as its device. True if it had none.
    pub(super) fn adopt(&mut self, id: u64) -> bool {
        let Some(a) = self.accounts.get(&id) else {
            return false;
        };
        let adopted = self.logins.adopt(a);
        if adopted {
            self.save_logins();
        }
        adopted
    }

    /// Save accounts.txt. False (and logged) if it couldn't be.
    pub(super) fn save_accounts(&self) -> bool {
        let path = self.dir.join("accounts.txt");
        match store::save(&path, self.accounts.values()) {
            Ok(()) => true,
            Err(e) => {
                log(format_args!("live: can't save {}: {e}", path.display()));
                false
            }
        }
    }

    /// Save logins.txt. False (and logged) if it couldn't be.
    fn save_logins(&self) -> bool {
        let path = self.dir.join("logins.txt");
        match self.logins.save(&path) {
            Ok(()) => true,
            Err(e) => {
                log(format_args!("live: can't save {}: {e}", path.display()));
                false
            }
        }
    }

    /// PC `k` proved it holds `login.key`: sign it in, or say why not.
    pub(super) fn prove(&mut self, k: usize, login: Login, now: f64) {
        let launcher = login.client.kind() == ClientKind::Launcher;
        if let Some(id) = self.logins.owner_of(&login.key) {
            return self.sign_in(k, id, login, false, now);
        }
        let id = store::account_id(&login.key);
        if self.accounts.contains_key(&id) {
            // Its PCs were signed out.
            let why = if launcher {
                live::PASSWORD_NEEDED
            } else {
                live::SIGNED_OUT
            };
            return self.refuse(k, why);
        }
        if self.sign_up_closed() {
            let why = if launcher {
                live::PASSWORD_NEEDED
            } else {
                live::NO_ACCOUNT
            };
            return self.refuse(k, why);
        }
        self.sign_in(k, id, login, false, now);
    }

    /// Passwords checked or hashed in the last minute.
    fn hashes_recent(&self, now: f64) -> usize {
        self.hashes.iter().filter(|&&t| now - t < 60.0).count()
    }

    /// Room for another hash now? If so, it's counted.
    fn take_hash(&mut self, now: f64) -> bool {
        if self.hashes_recent(now) >= HASHES_PER_MINUTE {
            return false;
        }
        self.hashes.push_back(now);
        true
    }

    fn wrong_for_account(&self, id: u64, now: f64) -> usize {
        let times = self.wrong_by_account.get(&id);
        times.map_or(0, |t| recent(t, now, WRONG_WINDOW))
    }

    fn wrong_from(&self, ip: IpAddr, now: f64) -> usize {
        let times = self.wrong_by_ip.get(&ip);
        times.map_or(0, |t| recent(t, now, WRONG_WINDOW))
    }

    /// Wrong passwords for `gamertag` (as `clean_name` makes it), which no
    /// account has, in the last 10 minutes.
    fn wrong_for_name(&self, gamertag: &str, now: f64) -> usize {
        let times = self.wrong_by_name.get(gamertag);
        times.map_or(0, |t| recent(t, now, WRONG_WINDOW))
    }

    /// Passwords checked to sign in to account `id` in the last 10 minutes.
    fn checks_for(&self, id: u64, now: f64) -> usize {
        let times = self.checks_by_account.get(&id);
        times.map_or(0, |t| recent(t, now, WRONG_WINDOW))
    }

    /// A wrong password, for `account` if it's one, from `ip`.
    fn wrong_password(&mut self, account: Option<u64>, ip: IpAddr, now: f64) {
        if let Some(id) = account {
            self.wrong_by_account.entry(id).or_default().push(now);
        }
        self.wrong_by_ip.entry(ip).or_default().push(now);
    }

    /// Drop the times of hashes and wrong passwords too old to count.
    pub(super) fn forget_old_tries(&mut self, now: f64) {
        while self.hashes.front().is_some_and(|&t| now - t >= 60.0) {
            self.hashes.pop_front();
        }
        for times in self.wrong_by_account.values_mut() {
            times.retain(|&t| now - t < WRONG_WINDOW);
        }
        for times in self.wrong_by_ip.values_mut() {
            times.retain(|&t| now - t < WRONG_WINDOW);
        }
        for times in self.wrong_by_name.values_mut() {
            times.retain(|&t| now - t < WRONG_WINDOW);
        }
        for times in self.checks_by_account.values_mut() {
            times.retain(|&t| now - t < WRONG_WINDOW);
        }
        for times in self.changes_by_account.values_mut() {
            times.retain(|&t| now - t < WRONG_WINDOW);
        }
        self.wrong_by_account.retain(|_, times| !times.is_empty());
        self.wrong_by_ip.retain(|_, times| !times.is_empty());
        self.wrong_by_name.retain(|_, times| !times.is_empty());
        self.checks_by_account.retain(|_, times| !times.is_empty());
        self.changes_by_account.retain(|_, times| !times.is_empty());
    }

    /// The passwords in `sealed` from PC `k`, if it opens (sealed for
    /// `purpose` and `target`, on this connection, by this PC) and holds
    /// `n` of them.
    fn open_box(
        &self,
        k: usize,
        purpose: u8,
        target: &[u8],
        sealed: &Sealed,
        n: usize,
    ) -> Option<Vec<Secret>> {
        let pc = &self.pcs[k];
        let aad = seal::aad(purpose, &pc.nonce, &pc.key, target);
        let plain = seal::open(pc.seal.as_ref()?, &aad, sealed)?;
        seal::unpad(&plain).filter(|p| p.len() == n)
    }

    /// A launcher on PC `k` proved it holds `login.key` and sent the
    /// password of the account named in its LOGIN: sign it in, its key a
    /// device of the account from now on, or say why not.
    pub(super) fn prove_password(
        &mut self,
        k: usize,
        login: Login,
        computer: &str,
        sealed: &Sealed,
        now: f64,
    ) {
        let ip = self.pcs[k].ip;
        let wanted = clean_name(&login.gamertag);
        let found = self.find(&wanted);
        // A PC the account knows signs in as PROVE signs it in: its
        // password isn't checked, so it costs no hash and no lock stops it.
        if let Some(a) = found.filter(|&a| self.logins.owner_of(&login.key) == Some(a)) {
            return self.sign_in(k, a, login, false, now);
        }
        if self.hashes_recent(now) >= HASHES_PER_MINUTE {
            return self.refuse(k, live::SERVER_BUSY);
        }
        if self.wrong_from(ip, now) >= WRONG_PER_IP {
            return self.refuse(k, live::TOO_MANY_TRIES);
        }
        // Locked, or checked too often: no hash, and it isn't counted (so
        // the lock isn't made longer). A gamertag no account has locks as
        // an account does, so the lock doesn't tell which are accounts.
        let locked = match found {
            Some(a) => {
                self.wrong_for_account(a, now) >= WRONG_PER_ACCOUNT
                    || self.checks_for(a, now) >= CHECKS_PER_ACCOUNT
            }
            None => self.wrong_for_name(&wanted, now) >= WRONG_PER_ACCOUNT,
        };
        if locked {
            return self.refuse(k, live::TOO_MANY_TRIES);
        }
        let typed = self.open_box(k, seal::SIGN_IN, login.gamertag.as_bytes(), sealed, 1);
        let phc = found.and_then(|a| self.logins.get(a)?.password.clone());
        // A box that opens costs one hash whatever it names: with no
        // password to check (a gamertag no account has, or an account
        // without one), the stand-in is checked, and the answer thrown
        // away, so the answer takes as long. (A box that doesn't open costs
        // none, whatever it names.)
        let right = match (found, &phc, &typed) {
            (Some(a), Some(phc), Some(typed)) => {
                // Right or wrong, it counts for the account.
                self.hashes.push_back(now);
                self.checks_by_account.entry(a).or_default().push(now);
                password::verify(phc, typed[0].as_str())
            }
            (_, _, Some(typed)) => {
                self.hashes.push_back(now);
                let _ = password::verify(self.stand_in(), typed[0].as_str());
                false
            }
            (_, _, None) => false,
        };
        let Some(a) = found.filter(|_| right) else {
            self.wrong_password(found, ip, now);
            if found.is_none() {
                self.wrong_by_name
                    .entry(wanted.clone())
                    .or_default()
                    .push(now);
            }
            match found.and_then(|a| self.accounts.get(&a)) {
                Some(account) => log(format_args!(
                    "live: wrong password for {} from {ip} ({} in 10 minutes)",
                    account.gamertag,
                    self.wrong_for_account(account.id, now)
                )),
                // Not the name: it may be a password typed in the wrong
                // box (a password's dashes don't fit in a gamertag, and
                // don't count).
                None => log(format_args!(
                    "live: wrong password for a gamertag no account has, from {ip} \
                     ({} in 10 minutes)",
                    self.wrong_from(ip, now)
                )),
            }
            return self.refuse(k, live::WRONG_PASSWORD);
        };
        self.wrong_by_account.remove(&a);
        let new_pc = self.logins.owner_of(&login.key) != Some(a);
        if new_pc {
            let pcs = self.logins.get(a).map_or(0, |l| l.devices.len());
            if pcs >= MAX_PCS {
                return self.refuse(k, live::TOO_MANY_PCS);
            }
            let mut name: String = printable(computer.trim())
                .chars()
                .take(MAX_PC_NAME)
                .collect();
            if name.trim().is_empty() {
                name = "PC".into();
            }
            let device = Device {
                key: login.key,
                added: self.epoch + now as u64,
                name: name.trim().to_string(),
            };
            let shown = device.name.clone();
            if let Some(from) = self.logins.add_device(a, device) {
                // Its old account's session from this PC ends.
                if let Some(j) = self.pc_of(from).filter(|&j| j != k) {
                    if self.pcs[j].key == login.key {
                        self.refuse(j, live::SIGNED_OUT);
                    }
                }
            }
            self.save_logins();
            log(format_args!(
                "live: {} signed in with a password from {ip} (new PC \"{shown}\", {} of {MAX_PCS})",
                self.accounts[&a].gamertag,
                pcs + 1
            ));
        }
        self.sign_in(k, a, login, new_pc, now);
    }

    /// A launcher's account request (signed in as `me`, on PC `k`). False
    /// if it shouldn't have sent it: badly signed, numbered no higher than
    /// its last, an admin's request from someone who isn't one, or a box
    /// that doesn't open.
    pub(super) fn account_request(
        &mut self,
        k: usize,
        me: u64,
        seq: u32,
        op: AccountOp,
        sig: &[u8; 64],
        now: f64,
    ) -> bool {
        let pc = &self.pcs[k];
        let proof = live::account_proof(&pc.nonce, seq, &op);
        if seq <= pc.account_seq || !signed_over(&pc.key, &proof, sig) {
            return false;
        }
        self.pcs[k].account_seq = seq;
        let mine = matches!(op, AccountOp::ChangePassword { .. });
        if !mine && !self.logins.is_admin(me) {
            return false;
        }
        let by = Actor::Admin(me);
        let result = match op {
            AccountOp::List => Ok(String::new()),
            AccountOp::Create { gamertag, password } => {
                let target = gamertag.as_bytes();
                let Some(typed) = self.open_box(k, seal::CREATE, target, &password, 1) else {
                    return false;
                };
                self.check_new_name(&gamertag)
                    .and_then(|_| self.launcher_hash(typed[0].as_str(), now))
                    .and_then(|phc| {
                        let admin = false;
                        self.command(
                            by,
                            Command::Add {
                                gamertag,
                                phc,
                                admin,
                            },
                            now,
                        )
                    })
            }
            AccountOp::SetPassword { account, password } => {
                let target = account.to_le_bytes();
                let Some(typed) = self.open_box(k, seal::SET_PASSWORD, &target, &password, 1)
                else {
                    return false;
                };
                self.guard(by, account)
                    .and_then(|_| self.launcher_hash(typed[0].as_str(), now))
                    .and_then(|phc| self.command(by, Command::SetPassword { account, phc }, now))
            }
            AccountOp::SignOut { account } => self.command(by, Command::SignOut { account }, now),
            AccountOp::Remove { account } => self.command(by, Command::Remove { account }, now),
            AccountOp::ChangePassword { passwords } => {
                let target = me.to_le_bytes();
                let Some(typed) = self.open_box(k, seal::CHANGE_PASSWORD, &target, &passwords, 2)
                else {
                    return false;
                };
                self.change_password(k, me, &typed[0], &typed[1], now)
            }
        };
        let (ok, text) = match result {
            Ok(text) => (true, text),
            Err(text) => (false, text),
        };
        self.send_accounts(me, seq, ok, text);
        true
    }

    /// A launcher's password hashed (counted towards the server's hashes).
    fn launcher_hash(&mut self, typed: &str, now: f64) -> Result<String, String> {
        password::check(typed).map_err(String::from)?;
        if !self.take_hash(now) {
            return Err(live::SERVER_BUSY.into());
        }
        password::hash(typed, self.cost).map_err(String::from)
    }

    /// `gamertag` may be a new account's: it keeps the rules and no one has
    /// it. The name as it would be kept.
    fn check_new_name(&self, gamertag: &str) -> Result<String, String> {
        let name = password::check_gamertag(gamertag).map_err(String::from)?;
        if self.find(&name).is_some() {
            return Err(live::GAMERTAG_TAKEN.into());
        }
        Ok(name)
    }

    /// `by` may change `account`: it's there, and an admin on a launcher
    /// changes neither their own account nor another admin's.
    fn guard(&self, by: Actor, account: u64) -> Result<(), String> {
        if !self.accounts.contains_key(&account) {
            return Err(NO_SUCH_ACCOUNT.into());
        }
        if let Actor::Admin(me) = by {
            if account == me {
                return Err(YOUR_OWN.into());
            }
            if self.logins.is_admin(account) {
                return Err(AN_ADMINS.into());
            }
        }
        Ok(())
    }

    /// A launcher player changes their own password: `old` must be right.
    /// Wrong ones count as wrong passwords do (for the account and the
    /// address), and right ones `CHANGES_PER_ACCOUNT` at most in 10
    /// minutes.
    fn change_password(
        &mut self,
        k: usize,
        me: u64,
        old: &Secret,
        new: &Secret,
        now: f64,
    ) -> Result<String, String> {
        let Some(phc) = self.logins.get(me).and_then(|l| l.password.clone()) else {
            return Err(NO_PASSWORD_YET.into());
        };
        let ip = self.pcs[k].ip;
        if self.wrong_for_account(me, now) >= WRONG_PER_ACCOUNT
            || self.wrong_from(ip, now) >= WRONG_PER_IP
        {
            return Err(live::TOO_MANY_TRIES.into());
        }
        let changes = self.changes_by_account.get(&me);
        if changes.map_or(0, |t| recent(t, now, WRONG_WINDOW)) >= CHANGES_PER_ACCOUNT {
            return Err(TOO_MANY_CHANGES.into());
        }
        password::check(new.as_str()).map_err(String::from)?;
        // Room for both its hashes (the old password checked, the new one
        // made), so none is spent on a change that can't be made.
        if self.hashes_recent(now) + 2 > HASHES_PER_MINUTE {
            return Err(live::SERVER_BUSY.into());
        }
        self.hashes.push_back(now);
        let gamertag = self.gamertag(me);
        if !password::verify(&phc, old.as_str()) {
            self.wrong_password(Some(me), ip, now);
            log(format_args!(
                "live: wrong password for {gamertag} from {ip} changing it ({} in 10 minutes)",
                self.wrong_for_account(me, now)
            ));
            return Err(WRONG_OLD_PASSWORD.into());
        }
        self.wrong_by_account.remove(&me);
        let phc = self.launcher_hash(new.as_str(), now)?;
        // Both hashes are spent: it counts, saved or not.
        self.changes_by_account.entry(me).or_default().push(now);
        self.logins.set_password(me, phc);
        log(format_args!("live: {gamertag} changed their password"));
        if !self.save_logins() {
            // It shuts the old password out, so it holds (as an admin's
            // new password does), and says so: the new one works now.
            return Err(format!("{PASSWORD_CHANGED} {NOT_SAVED}"));
        }
        Ok(PASSWORD_CHANGED.into())
    }

    /// Who `by` is, for the log.
    fn who(&self, by: Actor) -> String {
        match by {
            Actor::Console => "command line".into(),
            Actor::Admin(me) => self.gamertag(me),
        }
    }

    /// Make, change or remove an account, as the launcher's ACCOUNTS screen
    /// and the command line ask. What to show, or why it was refused.
    ///
    /// A change logins.txt can't be saved with is undone and refused
    /// (CANT_SAVE) if it would let anyone in or keep an account; one that
    /// shuts someone out holds until the server next starts, and is
    /// refused with what it did and NOT_SAVED, so it's asked for again. A
    /// new account or name accounts.txt can't be saved with is undone and
    /// refused (CANT_SAVE).
    pub fn command(&mut self, by: Actor, cmd: Command, now: f64) -> Result<String, String> {
        let who = self.who(by);
        let mut saved = true;
        let done = match cmd {
            Command::Add {
                gamertag,
                phc,
                admin,
            } => {
                let name = self.check_new_name(&gamertag)?;
                if !password::is_phc(&phc) {
                    return Err(NOT_A_HASH.into());
                }
                if admin && by != Actor::Console {
                    return Err(ONLY_THE_COMMAND_LINE.into());
                }
                let created = self.epoch + now as u64;
                let taken = |id| self.accounts.contains_key(&id) || self.logins.get(id).is_some();
                let account = Account::named(name.clone(), created, taken);
                let id = account.id;
                // Sign-ins first: if accounts.txt never gets the account,
                // they're dropped at the next start.
                self.logins.create(id, admin, Some(phc));
                if !self.save_logins() {
                    self.logins.remove(id);
                    return Err(CANT_SAVE.into());
                }
                // Then accounts.txt: unsaved, the account would be gone at
                // the next start, so it's undone (its sign-ins too).
                let mut account = account;
                account.seq += 1;
                self.accounts.insert(id, account);
                if !self.save_accounts() {
                    self.accounts.remove(&id);
                    self.logins.remove(id);
                    // (Unsaved, its sign-ins are dropped at the next start
                    // all the same.)
                    self.save_logins();
                    return Err(CANT_SAVE.into());
                }
                let what = if admin { "admin account" } else { "account" };
                log(format_args!("live: {who}: made the {what} {name}"));
                format!("CREATED {name}")
            }
            Command::SetPassword { account, phc } => {
                self.guard(by, account)?;
                if !password::is_phc(&phc) {
                    return Err(NOT_A_HASH.into());
                }
                self.logins.set_password(account, phc);
                self.wrong_by_account.remove(&account);
                self.checks_by_account.remove(&account);
                saved = self.save_logins();
                let name = self.gamertag(account);
                log(format_args!("live: {who} gave {name} a new password"));
                format!("{name} HAS A NEW PASSWORD. THEIR PCS STAY SIGNED IN.")
            }
            Command::SignOut { account } => {
                self.guard(by, account)?;
                let pcs = self.logins.sign_out(account);
                saved = self.save_logins();
                if let Some(j) = self.pc_of(account) {
                    self.refuse(j, live::SIGNED_OUT);
                }
                let name = self.gamertag(account);
                log(format_args!(
                    "live: {who} signed {name} out of all PCs ({})",
                    count(pcs, "PC", "PCs")
                ));
                let mut text = format!("{name} IS SIGNED OUT OF ALL PCS.");
                if self
                    .logins
                    .get(account)
                    .is_none_or(|l| l.password.is_none())
                {
                    text += " GIVE THEM A PASSWORD SO THEY CAN SIGN IN AGAIN.";
                }
                text
            }
            Command::Remove { account } => {
                self.guard(by, account)?;
                if self.logins.is_admin(account) && self.logins.admins() == 1 {
                    return Err(LAST_ADMIN.into());
                }
                let name = self.gamertag(account);
                // accounts.txt without it before anything else: an account
                // left there without sign-ins would be adopted at the next
                // start, its name key signing in again. Unsaved, nothing
                // changes.
                let path = self.dir.join("accounts.txt");
                let kept = self.accounts.values().filter(|a| a.id != account);
                if let Err(e) = store::save(&path, kept) {
                    log(format_args!("live: can't save {}: {e}", path.display()));
                    return Err(CANT_SAVE.into());
                }
                // Out of its party, the matchmaker and its matches.
                if let Some(j) = self.pc_of(account) {
                    self.refuse(j, live::ACCOUNT_REMOVED);
                }
                self.accounts.remove(&account);
                let friends = self.friends.forget(account);
                if !friends.is_empty() {
                    let path = self.dir.join("friends.txt");
                    if let Err(e) = self.friends.save(&path) {
                        log(format_args!("live: can't save {}: {e}", path.display()));
                    }
                    for other in friends {
                        self.friends_changed_now(other);
                    }
                }
                self.logins.remove(account);
                self.wrong_by_account.remove(&account);
                self.checks_by_account.remove(&account);
                // (Unsaved, its sign-ins are dropped at the next start all
                // the same: accounts.txt hasn't the account.)
                self.save_logins();
                log(format_args!("live: {who} removed {name}"));
                format!("REMOVED {name}")
            }
            Command::SetAdmin { account, admin } => {
                if by != Actor::Console {
                    return Err(ONLY_THE_COMMAND_LINE.into());
                }
                self.guard(by, account)?;
                let was = self.logins.is_admin(account);
                if was && !admin && self.logins.admins() == 1 {
                    return Err(LAST_ADMIN.into());
                }
                let closed = self.sign_up_closed();
                self.logins.set_admin(account, admin);
                saved = self.save_logins();
                if !saved && admin {
                    // Unsaved, a new admin is no admin.
                    self.logins.set_admin(account, was);
                    return Err(CANT_SAVE.into());
                }
                let name = self.gamertag(account);
                let mut text = if admin {
                    format!("{name} IS AN ADMIN NOW.")
                } else {
                    format!("{name} ISN'T AN ADMIN NOW.")
                };
                if self
                    .logins
                    .get(account)
                    .is_none_or(|l| l.password.is_none())
                {
                    text += &format!(" {name} HAS NO PASSWORD YET.");
                }
                log(format_args!(
                    "live: {who}: {name} {} an admin",
                    if admin { "is" } else { "isn't" }
                ));
                if closed != self.sign_up_closed() {
                    for line in self.sign_up_line().lines() {
                        log(line);
                    }
                }
                text
            }
            Command::Rename { account, gamertag } => {
                if by != Actor::Console {
                    return Err(RENAMES.into());
                }
                self.guard(by, account)?;
                let name = password::check_gamertag(&gamertag).map_err(String::from)?;
                if !self.free(&name, account) {
                    return Err(live::GAMERTAG_TAKEN.into());
                }
                let mut a = self.accounts[&account].clone();
                let old = std::mem::replace(&mut a.gamertag, name.clone());
                if self.accounts.get(&account) != Some(&a) {
                    // Unsaved, the old name would come back at the next
                    // start: undone, and refused.
                    a.seq += 1;
                    let was = self.accounts.insert(account, a);
                    if !self.save_accounts() {
                        if let Some(was) = was {
                            self.accounts.insert(account, was);
                        }
                        return Err(CANT_SAVE.into());
                    }
                    self.welcome(account);
                    if let Some(k) = self.pc_of(account) {
                        self.party_changed(self.pcs[k].party);
                    }
                    self.online_changed = true;
                    // Every list it's on shows the new name, signed in or
                    // not (a presence changes only while it is).
                    let mut theirs = self.friends.friends_of(account);
                    theirs.extend(self.friends.asked_by(account));
                    theirs.extend(self.friends.asking(account));
                    for other in theirs {
                        self.friends_changed(other);
                    }
                }
                log(format_args!("live: {who} renamed {old} {name}"));
                format!("{old} IS {name} NOW")
            }
        };
        self.accounts_changed();
        if saved {
            Ok(done)
        } else {
            Err(format!("{done} {NOT_SAVED}"))
        }
    }

    /// Every launcher signed in is sent the accounts as it sees them now.
    pub(super) fn accounts_changed(&mut self) {
        let online: Vec<u64> = self
            .pcs
            .iter()
            .filter(|pc| pc.gone.is_none() && pc.client == ClientKind::Launcher)
            .filter_map(|pc| pc.account)
            .collect();
        for id in online {
            self.send_accounts(id, 0, true, String::new());
        }
    }

    /// Send `me` (if signed in on a launcher: never the game) ACCOUNTS,
    /// answering their request `answers` (0 for none).
    pub(super) fn send_accounts(&mut self, me: u64, answers: u32, ok: bool, text: String) {
        if self.client_of(me) != ClientKind::Launcher {
            return;
        }
        let admin = self.logins.is_admin(me);
        let mut rows = if admin {
            let mut rows = self.account_rows();
            rows.truncate(MAX_ACCOUNT_ROWS);
            rows
        } else {
            self.accounts
                .get(&me)
                .map(|a| self.row(a))
                .into_iter()
                .collect()
        };
        // Levels as launchers see them everywhere else (the command line's
        // list keeps every playlist's).
        for row in &mut rows {
            if let Some(a) = self.accounts.get(&row.account) {
                row.best = self.best_for(a, ClientKind::Launcher);
            }
        }
        let accounts = Accounts {
            admin,
            password: self.logins.get(me).is_some_and(|l| l.password.is_some()),
            answers,
            ok,
            text,
            total: self.accounts.len().min(usize::from(u16::MAX)) as u16,
            rows,
        };
        self.tell(me, &ToPc::Accounts(accounts));
    }
}
