//! The log: `%LOCALAPPDATA%\h2launch\h2launch.log`, also echoed to the
//! console. Every line is `[  12.345] [t 1234] message`, seconds since
//! start and the calling thread. Lines are written and flushed one at a
//! time so a crash loses nothing. The crash handler writes through
//! `raw`, which takes no lock and allocates nothing.

use std::fmt::Write as _;
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

static FILE: Mutex<Option<File>> = Mutex::new(None);
/// A second handle on the same file for the crash path.
static RAW: OnceLock<File> = OnceLock::new();
/// Lines logged before the file was open.
static EARLY: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Opens the log in `dir`, keeping the previous run's log as
/// `h2launch.prev.log`. Returns the log's path.
pub fn init(dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("h2launch.log");
    if path.exists() {
        let _ = std::fs::rename(&path, dir.join("h2launch.prev.log"));
    }
    let mut f = File::create(&path)?;
    if let Ok(early) = EARLY.lock() {
        for l in early.iter() {
            let _ = f.write_all(l.as_bytes());
        }
    }
    if let Ok(clone) = f.try_clone() {
        let _ = RAW.set(clone);
    }
    if let Ok(mut g) = FILE.lock() {
        *g = Some(f);
    }
    Ok(path)
}

fn prefix() -> String {
    format!("[{:9.3}] [t {:5}] ", super::now(), super::tid())
}

/// One line.
pub fn line(msg: &str) {
    let mut s = prefix();
    s.push_str(msg);
    s.push('\n');
    {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(s.as_bytes());
        let _ = out.flush();
    }
    // try_lock with a short retry: the crash path may run on a thread
    // that already holds the lock, where a plain lock would never return.
    for _ in 0..50 {
        match FILE.try_lock() {
            Ok(mut g) => {
                match g.as_mut() {
                    Some(f) => {
                        let _ = f.write_all(s.as_bytes());
                        let _ = f.flush();
                    }
                    None => {
                        if let Ok(mut e) = EARLY.try_lock() {
                            e.push(s);
                        }
                    }
                }
                return;
            }
            Err(std::sync::TryLockError::Poisoned(_)) => break,
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(std::time::Duration::from_millis(1))
            }
        }
    }
    raw(s.as_bytes());
}

/// Crash path: no lock, no allocation.
pub fn raw(bytes: &[u8]) {
    if let Some(f) = RAW.get() {
        let mut f: &File = f;
        let _ = f.write_all(bytes);
        let _ = f.flush();
    }
    let _ = std::io::stderr().write_all(bytes);
}

/// A fixed buffer to format crash lines into without the heap.
pub struct StackLine {
    buf: [u8; 1024],
    len: usize,
}

impl Default for StackLine {
    fn default() -> Self {
        StackLine::new()
    }
}

impl StackLine {
    pub fn new() -> StackLine {
        StackLine {
            buf: [0; 1024],
            len: 0,
        }
    }

    /// Starts a line with the usual prefix (without allocating).
    pub fn start() -> StackLine {
        let mut s = StackLine::new();
        let _ = write!(s, "[{:9.3}] [t {:5}] ", super::now(), super::tid());
        s
    }

    /// What has been written so far, as text.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.buf[..self.len]).into_owned()
    }

    /// Writes the line (adding the newline) and empties the buffer.
    pub fn emit(&mut self) {
        if self.len < self.buf.len() {
            self.buf[self.len] = b'\n';
            self.len += 1;
        } else {
            self.buf[self.len - 1] = b'\n';
        }
        raw(&self.buf[..self.len]);
        self.len = 0;
    }
}

impl std::fmt::Write for StackLine {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        // Keep one byte for the newline; cut long lines.
        let room = self.buf.len() - 1 - self.len;
        let n = s.len().min(room);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

/// `log!("...", args)`: one formatted line.
macro_rules! log {
    ($($t:tt)*) => {
        $crate::win::log::line(&format!($($t)*))
    };
}
pub(crate) use log;
