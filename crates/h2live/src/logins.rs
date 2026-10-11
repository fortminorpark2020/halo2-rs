//! Who may sign in to each account: `logins.txt` in the data folder, beside
//! accounts.txt (which stays as it was, so an older server still reads the
//! folder, and stat cards never carry anything secret).
//!
//! Each account has an admin flag, an argon2id password hash (or none
//! yet), and up to `MAX_PCS` "devices": the keys of the PCs that sign in to
//! it without a password. A key belongs to one account at most.
//!
//! ```text
//! # h2live sign-ins: who may sign in to the accounts in accounts.txt.
//! # Private: it holds password hashes. h2live rewrites it; edit it only while h2live is stopped.
//! u <id hex16> <admin 0|1> <PHC or ->
//! d <id hex16> <device key hex64> <added unix> <PC name, may hold spaces>
//! ```
//!
//! `#` lines and empty lines are skipped. A `d` line comes after its
//! account's `u` line. The file is written whole, only its owner may read
//! it, and nothing in it is ever logged.
//!
//! An account accounts.txt has with no `u` line (one an older server made,
//! say) is "adopted" when a folder opens: no password, not an admin, and
//! the key its account is named by as its one device ("OLD PC"), so it
//! signs in from its PC as before.

use crate::password;
use crate::store::{self, hex, unhex, Account};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Most PCs an account signs in from without a password.
pub const MAX_PCS: usize = 5;
/// What an adopted account's first device is called.
pub const OLD_PC: &str = "OLD PC";
/// logins.txt's first lines.
const HEADER: &str = "# h2live sign-ins: who may sign in to the accounts in accounts.txt.\n\
                      # Private: it holds password hashes. h2live rewrites it; edit it only \
                      while h2live is stopped.\n";

/// A PC that signs in to an account without a password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    /// The PC's public key (what LOGIN names it by).
    pub key: [u8; 32],
    /// When it was added (Unix time).
    pub added: u64,
    /// The PC's name, as it gave it.
    pub name: String,
}

/// Who may sign in to one account. (Its Debug doesn't show the hash.)
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Login {
    pub admin: bool,
    /// The password's argon2id hash, as a PHC string.
    pub password: Option<String>,
    pub devices: Vec<Device>,
}

impl std::fmt::Debug for Login {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("Login")
            .field("admin", &self.admin)
            .field("password", &self.password.as_ref().map(|_| ".."))
            .field("devices", &self.devices)
            .finish()
    }
}

/// Every account's sign-ins.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Logins {
    logins: BTreeMap<u64, Login>,
    /// Each device's account.
    devices: HashMap<[u8; 32], u64>,
}

impl Logins {
    /// Logins as `text` writes them.
    pub fn parse(text: &str) -> Result<Logins, String> {
        let mut logins = Logins::default();
        for (i, line) in text.lines().enumerate() {
            let at = |e: &str| format!("line {}: {e}", i + 1);
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("u ") {
                let words: Vec<&str> = rest.split(' ').collect();
                let [id, admin, phc] = words[..] else {
                    return Err(at("not a sign-in line"));
                };
                let id = id_from_hex(id).ok_or_else(|| at("bad id"))?;
                let admin = match admin {
                    "0" => false,
                    "1" => true,
                    _ => return Err(at("admin isn't 0 or 1")),
                };
                let password = match phc {
                    "-" => None,
                    phc if password::is_phc(phc) => Some(phc.to_string()),
                    _ => return Err(at("not a password hash")),
                };
                if logins.logins.contains_key(&id) {
                    return Err(at("an account listed twice"));
                }
                let login = Login {
                    admin,
                    password,
                    devices: Vec::new(),
                };
                logins.logins.insert(id, login);
            } else if let Some(rest) = line.strip_prefix("d ") {
                let mut words = rest.splitn(4, ' ');
                let (Some(id), Some(key), Some(added), Some(name)) =
                    (words.next(), words.next(), words.next(), words.next())
                else {
                    return Err(at("not a PC line"));
                };
                let id = id_from_hex(id).ok_or_else(|| at("bad id"))?;
                let key = unhex::<32>(key).ok_or_else(|| at("bad key"))?;
                let added = added.parse().map_err(|_| at("bad time"))?;
                if !logins.logins.contains_key(&id) {
                    return Err(at("a PC before its account's u line"));
                }
                if logins.devices.contains_key(&key) {
                    return Err(at("a PC listed twice"));
                }
                let device = Device {
                    key,
                    added,
                    name: name.to_string(),
                };
                logins.devices.insert(key, id);
                logins.logins.entry(id).or_default().devices.push(device);
            } else {
                return Err(at("not a u or d line"));
            }
        }
        Ok(logins)
    }

    /// logins.txt's text.
    pub fn text(&self) -> String {
        let mut text = HEADER.to_string();
        for (id, login) in &self.logins {
            let phc = login.password.as_deref().unwrap_or("-");
            text += &format!("u {id:016x} {} {phc}\n", u8::from(login.admin));
            for d in &login.devices {
                text += &format!("d {id:016x} {} {} {}\n", hex(&d.key), d.added, d.name);
            }
        }
        text
    }

    /// The logins in `path`; None if there's no such file.
    pub fn load(path: &Path) -> Result<Option<Logins>, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Logins::parse(&text)
                .map(Some)
                .map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Write them to `path` (logins.txt) whole, for its owner's eyes only.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        store::replace_private(path, &self.text())
    }

    /// Drop every account `exists` says accounts.txt doesn't have. How many
    /// were dropped.
    pub fn keep_only(&mut self, exists: impl Fn(u64) -> bool) -> usize {
        let gone: Vec<u64> = self
            .logins
            .keys()
            .copied()
            .filter(|&id| !exists(id))
            .collect();
        for &id in &gone {
            self.remove(id);
        }
        gone.len()
    }

    /// Give `a` sign-ins if it has none: no password, not an admin, and the
    /// key it's named by as its device (unless another account has that
    /// key already). True if it had none.
    pub fn adopt(&mut self, a: &Account) -> bool {
        if self.logins.contains_key(&a.id) {
            return false;
        }
        self.create(a.id, false, None);
        if !self.devices.contains_key(&a.key) {
            let device = Device {
                key: a.key,
                added: a.created,
                name: OLD_PC.into(),
            };
            self.add_device(a.id, device);
        }
        true
    }

    /// The account the PC with `key` signs in to without a password.
    pub fn owner_of(&self, key: &[u8; 32]) -> Option<u64> {
        self.devices.get(key).copied()
    }

    pub fn get(&self, id: u64) -> Option<&Login> {
        self.logins.get(&id)
    }

    /// (Its devices change through `add_device`, `sign_out` and `remove`
    /// only, which keep the keys' owners straight.)
    pub fn get_mut(&mut self, id: u64) -> Option<&mut Login> {
        self.logins.get_mut(&id)
    }

    /// Sign-ins for a new account `id`, with no PCs yet (or, if it has some
    /// already, those kept).
    pub fn create(&mut self, id: u64, admin: bool, phc: Option<String>) {
        let login = self.logins.entry(id).or_default();
        login.admin = admin;
        login.password = phc;
    }

    /// Let the PC `device` sign in to `id` without a password: moved from
    /// the account that had it, if one did (returned). Already `id`'s, it
    /// stays as it was.
    pub fn add_device(&mut self, id: u64, device: Device) -> Option<u64> {
        let from = self.devices.get(&device.key).copied();
        if from == Some(id) {
            return None;
        }
        if let Some(from) = from {
            if let Some(login) = self.logins.get_mut(&from) {
                login.devices.retain(|d| d.key != device.key);
            }
        }
        self.devices.insert(device.key, id);
        self.logins.entry(id).or_default().devices.push(device);
        from
    }

    /// Sign `id` out of every PC. How many there were.
    pub fn sign_out(&mut self, id: u64) -> usize {
        let Some(login) = self.logins.get_mut(&id) else {
            return 0;
        };
        let devices = std::mem::take(&mut login.devices);
        for d in &devices {
            self.devices.remove(&d.key);
        }
        devices.len()
    }

    pub fn remove(&mut self, id: u64) {
        self.sign_out(id);
        self.logins.remove(&id);
    }

    /// A new password hash for `id`.
    pub fn set_password(&mut self, id: u64, phc: String) {
        self.logins.entry(id).or_default().password = Some(phc);
    }

    pub fn set_admin(&mut self, id: u64, admin: bool) {
        self.logins.entry(id).or_default().admin = admin;
    }

    pub fn is_admin(&self, id: u64) -> bool {
        self.logins.get(&id).is_some_and(|l| l.admin)
    }

    /// How many accounts are admins.
    pub fn admins(&self) -> usize {
        self.logins.values().filter(|l| l.admin).count()
    }

    /// How many accounts have sign-ins.
    pub fn len(&self) -> usize {
        self.logins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.logins.is_empty()
    }
}

/// An id written as 16 hex digits.
fn id_from_hex(text: &str) -> Option<u64> {
    unhex::<8>(text).map(u64::from_be_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::password::{self, Cost};
    use crate::server::Server;
    use std::path::PathBuf;

    /// A folder of a test's own, removed afterwards.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let name = format!("h2live-logins-{name}-{}", std::process::id());
            let path = std::env::temp_dir().join(name);
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

    fn device(n: u8, name: &str) -> Device {
        Device {
            key: [n; 32],
            added: 1_700_000_000 + u64::from(n),
            name: name.into(),
        }
    }

    fn some_logins() -> Logins {
        let phc = password::hash("K7MP-4QR9-XW2T", Cost::Cheap).unwrap();
        let mut logins = Logins::default();
        logins.create(0x1111, true, Some(phc));
        logins.add_device(0x1111, device(1, "KAT'S DESKTOP"));
        logins.add_device(0x1111, device(2, "LAPTOP"));
        logins.create(0x2222, false, None);
        logins.add_device(0x2222, device(3, OLD_PC));
        logins.create(0x3333, false, None);
        logins
    }

    #[test]
    fn logins_read_back_as_written() {
        let logins = some_logins();
        let text = logins.text();
        assert!(text.starts_with(HEADER));
        assert!(text.contains("d 0000000000001111 0101"));
        assert!(text.contains(" KAT'S DESKTOP\n"));
        assert!(text.contains("u 0000000000003333 0 -\n"));
        let read = Logins::parse(&text).unwrap();
        assert_eq!(read, logins);
        assert_eq!(read.text(), text);
        assert_eq!(read.owner_of(&[2; 32]), Some(0x1111));
        assert_eq!(read.owner_of(&[3; 32]), Some(0x2222));
        assert_eq!(read.owner_of(&[4; 32]), None);
        assert!(read.is_admin(0x1111) && !read.is_admin(0x2222));
        assert_eq!((read.admins(), read.len()), (1, 3));
        assert_eq!(Logins::parse("").unwrap(), Logins::default());
        assert_eq!(
            Logins::parse("# just a note\n\n").unwrap(),
            Logins::default()
        );
        // Its Debug shows no hash.
        assert!(!format!("{read:?}").contains("argon2"));
    }

    #[test]
    fn broken_logins_say_which_line() {
        let phc = password::hash("K7MP-4QR9-XW2T", Cost::Cheap).unwrap();
        let u = "u 0000000000001111 0 -";
        let d = format!("d 0000000000001111 {} 5 PC", "01".repeat(32));
        for (text, error) in [
            (
                "x 0000000000001111 0 -".to_string(),
                "line 1: not a u or d line",
            ),
            (
                format!("{d}\n{u}"),
                "line 1: a PC before its account's u line",
            ),
            (format!("{u}\n{u}"), "line 2: an account listed twice"),
            (format!("{u}\n{d}\n{d}"), "line 3: a PC listed twice"),
            ("u 00000000000011 0 -".into(), "line 1: bad id"),
            ("u 000000000000111g 0 -".into(), "line 1: bad id"),
            (
                "u 0000000000001111 2 -".into(),
                "line 1: admin isn't 0 or 1",
            ),
            ("u 0000000000001111 0".into(), "line 1: not a sign-in line"),
            (
                "u 0000000000001111 0 - more".into(),
                "line 1: not a sign-in line",
            ),
            (
                "u 0000000000001111 0 hunter2".into(),
                "line 1: not a password hash",
            ),
            (
                format!("{u}\nd 0000000000001111 0101 5 PC"),
                "line 2: bad key",
            ),
            (
                format!("{u}\nd 0000000000001111 {} x PC", "01".repeat(32)),
                "line 2: bad time",
            ),
            (
                format!("{u}\nd 0000000000001111 {}", "01".repeat(32)),
                "line 2: not a PC line",
            ),
        ] {
            assert_eq!(Logins::parse(&text), Err(error.to_string()), "{text}");
        }
        // A hash is taken as it was written, and the error never shows it.
        let line = format!("u 0000000000001111 1 {phc}");
        assert!(Logins::parse(&line).is_ok());
        let broken = format!("{line}\nu 0000000000001111 1 {phc}");
        assert!(!Logins::parse(&broken).unwrap_err().contains("argon2"));
    }

    #[test]
    fn devices_move_between_accounts_and_sign_out() {
        let mut logins = some_logins();
        // Already its own: nothing moves.
        assert_eq!(logins.add_device(0x1111, device(1, "AGAIN")), None);
        assert_eq!(logins.get(0x1111).unwrap().devices[0].name, "KAT'S DESKTOP");
        // Another account's: taken from it.
        assert_eq!(logins.add_device(0x3333, device(2, "LAPTOP")), Some(0x1111));
        assert_eq!(logins.owner_of(&[2; 32]), Some(0x3333));
        assert_eq!(logins.get(0x1111).unwrap().devices.len(), 1);
        assert_eq!(logins.sign_out(0x1111), 1);
        assert_eq!(logins.owner_of(&[1; 32]), None);
        assert!(logins.get(0x1111).unwrap().password.is_some());
        assert_eq!(logins.sign_out(0x9999), 0);
        logins.remove(0x3333);
        assert_eq!(logins.owner_of(&[2; 32]), None);
        assert!(logins.get(0x3333).is_none());
        logins.set_admin(0x2222, true);
        assert_eq!(logins.admins(), 2);
        logins.set_password(0x2222, password::hash("ANOTHER-ONE", Cost::Cheap).unwrap());
        assert!(logins.get(0x2222).unwrap().password.is_some());
        // What's left reads back the same.
        assert_eq!(Logins::parse(&logins.text()).unwrap(), logins);
    }

    #[test]
    fn only_accounts_there_are_are_kept() {
        let mut logins = some_logins();
        assert_eq!(logins.keep_only(|id| id != 0x2222), 1);
        assert!(logins.get(0x2222).is_none());
        assert_eq!(logins.owner_of(&[3; 32]), None);
        assert_eq!(logins.keep_only(|_| true), 0);
        assert_eq!(logins.len(), 2);
    }

    #[test]
    fn an_account_without_sign_ins_is_adopted_from_its_pc() {
        let mut logins = some_logins();
        let mut a = Account::new([7; 32], 1_700_000_777);
        a.gamertag = "OLD TIMER".into();
        assert!(logins.adopt(&a));
        let login = logins.get(a.id).unwrap();
        assert!(!login.admin && login.password.is_none());
        assert_eq!(
            login.devices,
            [device(7, OLD_PC)].map(|d| Device {
                added: 1_700_000_777,
                ..d
            })
        );
        assert_eq!(logins.owner_of(&[7; 32]), Some(a.id));
        // Only once.
        assert!(!logins.adopt(&a));
        // A key another account has already stays with it.
        let b = Account::new([1; 32], 5);
        assert!(logins.adopt(&b));
        assert!(logins.get(b.id).unwrap().devices.is_empty());
        assert_eq!(logins.owner_of(&[1; 32]), Some(0x1111));
    }

    #[test]
    fn logins_are_saved_for_their_owner_only() {
        let dir = TempDir::new("private");
        let path = dir.0.join("logins.txt");
        assert_eq!(Logins::load(&path), Ok(None));
        let logins = some_logins();
        logins.save(&path).unwrap();
        assert_eq!(Logins::load(&path), Ok(Some(logins)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::write(&path, "nonsense\n").unwrap();
        let error = Logins::load(&path).unwrap_err();
        assert!(error.contains("logins.txt: line 1"), "{error}");
    }

    #[test]
    fn an_older_server_reads_accounts_the_owner_makes() {
        let taken = |id| id == 7;
        let a = Account::named("KAT".into(), 1_700_000_000, taken);
        assert!(![0, 1, 7, u64::MAX - 1, u64::MAX].contains(&a.id));
        assert_eq!(a.id, store::account_id(&a.key));
        let read = store::parse(&a.lines()).unwrap();
        assert_eq!(read, std::slice::from_ref(&a));
        let b = Account::named("JUN".into(), 1_700_000_000, |id| id == a.id);
        assert_ne!((a.id, a.key), (b.id, b.key));
    }

    #[test]
    fn a_folder_from_an_older_server_keeps_its_accounts_as_they_were() {
        let dir = TempDir::new("older");
        let mut a = Account::new([1; 32], 1_700_000_001);
        a.gamertag = "KAT".into();
        a.seq = 4;
        let mut b = Account::new([2; 32], 1_700_000_002);
        b.gamertag = "JUN".into();
        b.seq = 9;
        let accounts = dir.0.join("accounts.txt");
        store::save(&accounts, [&a, &b]).unwrap();
        let before = std::fs::read(&accounts).unwrap();
        let server = Server::open(&dir.0, Some("a secret")).unwrap();
        // Every account signs in from its PC as before.
        let logins = Logins::load(&dir.0.join("logins.txt")).unwrap().unwrap();
        assert_eq!(logins.owner_of(&[1; 32]), Some(a.id));
        assert_eq!(logins.owner_of(&[2; 32]), Some(b.id));
        assert_eq!(logins.admins(), 0);
        assert_eq!(std::fs::read(&accounts).unwrap(), before);
        drop(server);
        // And again, with nothing to adopt.
        let logins_text = std::fs::read(dir.0.join("logins.txt")).unwrap();
        let _server = Server::open(&dir.0, Some("a secret")).unwrap();
        assert_eq!(std::fs::read(&accounts).unwrap(), before);
        assert_eq!(
            std::fs::read(dir.0.join("logins.txt")).unwrap(),
            logins_text
        );
    }

    #[test]
    fn a_folder_with_no_accounts_gets_no_logins_file() {
        let dir = TempDir::new("empty");
        let _server = Server::open(&dir.0, Some("a secret")).unwrap();
        assert!(!dir.0.join("logins.txt").exists());
    }
}
