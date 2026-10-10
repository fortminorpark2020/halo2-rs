//! Log lines written by a thread of their own, so the relay's thread never
//! waits on its output. On Windows a console with text selected in it
//! (QuickEdit) stops taking output until the selection ends; a relay that
//! printed there itself would stop passing packets at its next line.

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, OnceLock};

/// Where log lines go: called on the relay's thread, so it must not wait.
pub type Logger = Arc<dyn Fn(&str) + Send + Sync>;

/// Lines waiting to be written, at most; more are dropped and counted.
const QUEUE: usize = 1024;

/// A logger that hands each line to a thread writing them to `out`, one
/// per line. Never waits: when `queue` lines are waiting already, the line
/// is dropped, and the next one that fits says how many were.
pub fn background_logger<W: Write + Send + 'static>(out: W, queue: usize) -> Logger {
    let lines = writer(out, queue);
    let dropped = AtomicU64::new(0);
    Arc::new(move |line: &str| send(&lines, &dropped, line))
}

/// A logger for standard output (one writer thread for the whole program,
/// started when first asked for).
pub fn stdout_logger() -> Logger {
    static STDOUT: OnceLock<(SyncSender<String>, AtomicU64)> = OnceLock::new();
    Arc::new(|line: &str| {
        let (lines, dropped) =
            STDOUT.get_or_init(|| (writer(std::io::stdout(), QUEUE), AtomicU64::new(0)));
        send(lines, dropped, line);
    })
}

/// A thread writing the lines sent to it to `out`, until every sender is
/// gone.
fn writer<W: Write + Send + 'static>(mut out: W, queue: usize) -> SyncSender<String> {
    let (tx, rx) = mpsc::sync_channel::<String>(queue);
    let spawned = std::thread::Builder::new()
        .name("h2relay-log".into())
        .spawn(move || {
            for line in rx {
                // Nowhere to say it went wrong: carry on.
                let _ = writeln!(out, "{line}");
                let _ = out.flush();
            }
        });
    // Without a thread (the system has none to give), lines go nowhere:
    // `rx` is dropped with the failed closure, and sends just fail.
    drop(spawned);
    tx
}

fn send(lines: &SyncSender<String>, dropped: &AtomicU64, line: &str) {
    let missed = dropped.swap(0, Relaxed);
    if missed > 0 {
        let note = format!("({missed} log lines dropped: the output was held up)");
        if lines.try_send(note).is_err() {
            dropped.fetch_add(missed + 1, Relaxed);
            return;
        }
    }
    match lines.try_send(line.to_string()) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) => {
            dropped.fetch_add(1, Relaxed);
        }
        Err(TrySendError::Disconnected(_)) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// Output that takes nothing until let go, then keeps what it's given.
    #[derive(Clone)]
    struct Held {
        gate: Arc<Mutex<()>>,
        got: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for Held {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let _held = self.gate.lock().unwrap();
            self.got.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn held_up_output_never_holds_up_the_caller() {
        let out = Held {
            gate: Arc::new(Mutex::new(())),
            got: Arc::new(Mutex::new(Vec::new())),
        };
        let held = out.gate.lock().unwrap();
        let log = background_logger(out.clone(), 8);
        // Far more than fits, with the output stuck: none of it waits.
        let start = Instant::now();
        for i in 0..10_000 {
            log(&format!("line {i}"));
        }
        assert!(start.elapsed() < Duration::from_secs(2));
        // Let go: what fitted comes out, then a note of what didn't.
        drop(held);
        let wait = Instant::now();
        while !String::from_utf8_lossy(&out.got.lock().unwrap()).contains("line 0\n") {
            assert!(wait.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(1));
        }
        log("after");
        let text = loop {
            let text = String::from_utf8_lossy(&out.got.lock().unwrap()).into_owned();
            if text.contains("after") {
                break text;
            }
            assert!(wait.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(1));
        };
        assert!(text.contains("log lines dropped"), "{text}");
        assert!(!text.contains("line 9999\n"), "{text}");
        assert!(text.ends_with("after\n"), "{text}");
    }
}
