//! The scene behind the lobby's start screen and main menu, on a thread
//! of its own: it reads mainmenu.map (`load`), opens a GPU device
//! (`render::Renderer`) and draws the frames the lobby asks for, keeping
//! only the newest for the lobby to take. Asleep it lets go of every GPU
//! resource but keeps the scene's data, so it wakes again in well under a
//! second; the lobby puts it to sleep on every other screen and while a
//! match is played, so the engine has the GPU to itself.
//!
//! The lobby sees it through `SceneSource`, which its tests fake.

use crate::data::SceneData;
use crate::load::load;
use crate::render::{Eye, Renderer, Veil};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Where the scene's lines go (the lobby's log).
pub type Log = Arc<dyn Fn(&str) + Send + Sync>;

/// Frames drawn at a size before their times are logged, after the first
/// (which makes the frame's buffers).
const TIMED_FRAMES: u32 = 30;

/// A frame of the scene: `w` by `h` pixels, top row first, each
/// 0x00RRGGBB in sRGB, `seconds` into the menus.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub w: usize,
    pub h: usize,
    pub seconds: f64,
    pub px: Vec<u32>,
}

/// How the scene is getting on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneState {
    /// Reading mainmenu.map.
    Loading,
    /// Read: frames come when asked for (the first after a wake once the
    /// GPU device is open again).
    Ready,
    /// It can't be drawn, and won't be this run: why.
    Failed(String),
}

/// The scene as the lobby sees it.
pub trait SceneSource: Send + Sync {
    /// Ask for a frame `w` by `h`, `seconds` into the menus (the flythrough
    /// loops). It wakes the scene if it sleeps.
    fn ask(&self, w: usize, h: usize, seconds: f64);
    /// The newest frame drawn, if it is `w` by `h`.
    fn latest(&self, w: usize, h: usize) -> Option<Arc<Frame>>;
    /// Stop drawing and let go of the GPU until the next `ask`.
    fn sleep(&self);
    fn state(&self) -> SceneState;
}

/// A one-slot mailbox: what is put replaces what was there, so the reader
/// only ever sees the newest.
pub struct Mailbox<T> {
    slot: Mutex<Option<Arc<T>>>,
}

impl<T> Default for Mailbox<T> {
    fn default() -> Self {
        Mailbox {
            slot: Mutex::new(None),
        }
    }
}

impl<T> Mailbox<T> {
    /// Put `value` in, giving back what it replaced.
    pub fn put(&self, value: T) -> Option<Arc<T>> {
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        slot.replace(Arc::new(value))
    }

    /// The newest value, left in place.
    pub fn peek(&self) -> Option<Arc<T>> {
        self.slot.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// How the scene is drawn.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// A software adapter (lavapipe, WARP) may draw it: for tests and
    /// checks in a container (the lobby's `H2LOBBY_SCENE=on`).
    pub allow_cpu: bool,
    pub veil: Veil,
}

/// What the scene thread is asked to do.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ask {
    Frame { w: u32, h: u32, seconds: f64 },
    Sleep,
    Quit,
}

/// What the lobby's thread and the scene's share.
struct Shared {
    state: Mutex<SceneState>,
    frames: Mailbox<Frame>,
}

impl Shared {
    fn set(&self, state: SceneState) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = state;
    }
}

/// The scene, drawn on a thread of its own (see the module's words).
pub struct Backdrop {
    tx: Sender<Ask>,
    shared: Arc<Shared>,
}

impl Backdrop {
    /// Start reading `mainmenu` (Halo 2 Vista's mainmenu.map) and drawing
    /// it when asked. Its lines go to `log`: what was read and how long it
    /// took, the GPU it draws on, its sleeps and wakes, and its frames'
    /// times once per size. A failure is only its state (the lobby logs
    /// it).
    pub fn start(mainmenu: PathBuf, options: Options, log: Log) -> Backdrop {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared {
            state: Mutex::new(SceneState::Loading),
            frames: Mailbox::default(),
        });
        let s = shared.clone();
        let spawned = std::thread::Builder::new()
            .name("menu scene".into())
            .spawn(move || {
                let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run(&mainmenu, options, &log, &rx, &s)
                }));
                match run {
                    Ok(Ok(())) => {}
                    Ok(Err(why)) => s.set(SceneState::Failed(why)),
                    Err(_) => s.set(SceneState::Failed("the scene's thread failed".into())),
                }
            });
        if let Err(e) = spawned {
            shared.set(SceneState::Failed(format!("no thread for the scene: {e}")));
        }
        Backdrop { tx, shared }
    }
}

impl Drop for Backdrop {
    fn drop(&mut self) {
        // It stops after the frame it is drawing; nothing waits for it.
        let _ = self.tx.send(Ask::Quit);
    }
}

impl SceneSource for Backdrop {
    fn ask(&self, w: usize, h: usize, seconds: f64) {
        let (Ok(w), Ok(h)) = (u32::try_from(w), u32::try_from(h)) else {
            return;
        };
        if w > 0 && h > 0 {
            let _ = self.tx.send(Ask::Frame { w, h, seconds });
        }
    }

    fn latest(&self, w: usize, h: usize) -> Option<Arc<Frame>> {
        self.shared.frames.peek().filter(|f| f.w == w && f.h == h)
    }

    fn sleep(&self) {
        let _ = self.tx.send(Ask::Sleep);
    }

    fn state(&self) -> SceneState {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// The newest of the asks waiting after `first`: the asks before it are
/// out of date. None to quit (asked, or the lobby has gone).
fn newest(first: Ask, rx: &Receiver<Ask>) -> Option<Ask> {
    let mut ask = first;
    loop {
        if ask == Ask::Quit {
            return None;
        }
        match rx.try_recv() {
            Ok(next) => ask = next,
            Err(mpsc::TryRecvError::Empty) => return Some(ask),
            Err(mpsc::TryRecvError::Disconnected) => return None,
        }
    }
}

/// Frame times at one size, until they are logged.
struct Timing {
    size: (u32, u32),
    frames: u32,
    render: f64,
    readback: f64,
}

/// The scene's thread: read the map, then draw what is asked until told
/// to quit. An error is why the scene can't be drawn this run.
fn run(
    path: &Path,
    options: Options,
    log: &Log,
    rx: &Receiver<Ask>,
    shared: &Shared,
) -> Result<(), String> {
    let started = Instant::now();
    let warn = |line: &str| log(&format!("scene: {line}"));
    let data = load(path, &warn)?;
    let (level, objects, sky) = data.triangles();
    log(&format!(
        "scene: {} read in {:.1} s ({} triangles, {} textures, a {:.1} s flythrough)",
        path.display(),
        started.elapsed().as_secs_f64(),
        level + objects + sky,
        data.textures.len(),
        data.flythrough.seconds()
    ));
    shared.set(SceneState::Ready);
    let mut renderer: Option<Renderer> = None;
    let mut adapter_logged = false;
    let mut timing: Option<Timing> = None;
    let mut logged: Vec<(u32, u32)> = Vec::new();
    loop {
        let Ok(first) = rx.recv() else {
            return Ok(());
        };
        let Some(ask) = newest(first, rx) else {
            return Ok(());
        };
        match ask {
            Ask::Quit => return Ok(()),
            Ask::Sleep => {
                if renderer.take().is_some() {
                    log("scene: asleep (its GPU device closed)");
                }
            }
            Ask::Frame { w, h, seconds } => {
                let r = match &mut renderer {
                    Some(r) => r,
                    None => {
                        let woke = Instant::now();
                        let r = wake(&data, options)?;
                        if !adapter_logged {
                            log(&format!("scene: drawing on {}", r.adapter()));
                            adapter_logged = true;
                        }
                        log(&format!(
                            "scene: awake in {:.0} ms",
                            woke.elapsed().as_secs_f64() * 1000.0
                        ));
                        renderer.insert(r)
                    }
                };
                let length = data.flythrough.seconds().max(1e-3);
                let eye = Eye::flythrough(&data.flythrough, seconds.rem_euclid(length) as f32);
                let frame = r.render(w, h, eye)?;
                time(&mut timing, &mut logged, (w, h), &frame, log);
                let px = frame.px;
                let old = shared.frames.put(Frame {
                    w: w as usize,
                    h: h as usize,
                    seconds,
                    px,
                });
                // The frame before, if the lobby has let go of it, is
                // drawn into next.
                if let Some(old) = old.and_then(|f| Arc::try_unwrap(f).ok()) {
                    r.recycle(old.px);
                }
            }
        }
    }
}

/// A renderer with the scene on it.
fn wake(data: &SceneData, options: Options) -> Result<Renderer, String> {
    let mut r = Renderer::new(options.allow_cpu, options.veil)?;
    r.upload(data.parts())?;
    Ok(r)
}

/// Count a frame's times at `size`, and log their averages once per size
/// (`TIMED_FRAMES` frames after the first).
fn time(
    timing: &mut Option<Timing>,
    logged: &mut Vec<(u32, u32)>,
    size: (u32, u32),
    frame: &crate::render::Rendered,
    log: &Log,
) {
    if logged.contains(&size) {
        return;
    }
    let t = match timing {
        Some(t) if t.size == size => t,
        _ => {
            // The first frame at a size makes its buffers: not counted.
            *timing = Some(Timing {
                size,
                frames: 0,
                render: 0.0,
                readback: 0.0,
            });
            return;
        }
    };
    t.frames += 1;
    t.render += frame.render_ms;
    t.readback += frame.readback_ms;
    if t.frames == TIMED_FRAMES {
        let n = f64::from(TIMED_FRAMES);
        log(&format!(
            "scene: {}x{}, {:.1} ms to draw a frame and {:.1} ms to read it back (on its own thread)",
            size.0,
            size.1,
            t.render / n,
            t.readback / n
        ));
        logged.push(size);
        *timing = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_mailbox_keeps_only_the_newest() {
        let m = Mailbox::default();
        assert!(m.peek().is_none());
        assert!(m.put(1).is_none());
        assert_eq!(m.put(2).as_deref(), Some(&1));
        assert_eq!(m.put(3).as_deref(), Some(&2));
        // Looking leaves it there.
        assert_eq!(m.peek().as_deref(), Some(&3));
        assert_eq!(m.peek().as_deref(), Some(&3));
        // What was replaced can be had back whole once nobody holds it.
        let held = m.peek();
        let old = m.put(4).unwrap();
        assert!(Arc::try_unwrap(old).is_err());
        drop(held);
        assert_eq!(Arc::try_unwrap(m.put(5).unwrap()).ok(), Some(4));
    }

    #[test]
    fn only_the_newest_ask_is_drawn() {
        let (tx, rx) = mpsc::channel();
        let frame = |s| Ask::Frame {
            w: 4,
            h: 4,
            seconds: s,
        };
        for s in [1.0, 2.0, 3.0] {
            tx.send(frame(s)).unwrap();
        }
        let first = rx.recv().unwrap();
        assert_eq!(newest(first, &rx), Some(frame(3.0)));
        // A sleep after a frame wins; a frame after a sleep wakes it.
        tx.send(frame(4.0)).unwrap();
        tx.send(Ask::Sleep).unwrap();
        let first = rx.recv().unwrap();
        assert_eq!(newest(first, &rx), Some(Ask::Sleep));
        tx.send(Ask::Sleep).unwrap();
        tx.send(frame(5.0)).unwrap();
        let first = rx.recv().unwrap();
        assert_eq!(newest(first, &rx), Some(frame(5.0)));
        // Quit, wherever it is, quits.
        tx.send(Ask::Quit).unwrap();
        tx.send(frame(6.0)).unwrap();
        let first = rx.recv().unwrap();
        assert_eq!(newest(first, &rx), None);
        // As does the lobby going, after what it asked last.
        tx.send(frame(7.0)).unwrap();
        tx.send(frame(8.0)).unwrap();
        drop(tx);
        let first = rx.recv().unwrap();
        assert_eq!(newest(first, &rx), None);
    }

    #[test]
    fn a_missing_map_fails_and_says_why() {
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let kept = lines.clone();
        let log: Log = Arc::new(move |l: &str| kept.lock().unwrap().push(l.into()));
        let options = Options {
            allow_cpu: false,
            veil: Veil::NONE,
        };
        let b = Backdrop::start(PathBuf::from("/nowhere/mainmenu.map"), options, log);
        let until = Instant::now() + Duration::from_secs(10);
        while b.state() == SceneState::Loading && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        match b.state() {
            SceneState::Failed(why) => assert!(why.contains("mainmenu.map"), "{why}"),
            s => panic!("{s:?}"),
        }
        // Asking after does nothing, and no frame comes.
        b.ask(64, 36, 1.0);
        b.sleep();
        assert!(b.latest(64, 36).is_none());
        // The thread logs no failure of its own: the lobby does, once.
        assert!(lines.lock().unwrap().is_empty());
    }
}
