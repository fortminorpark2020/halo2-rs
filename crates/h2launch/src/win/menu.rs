//! The in-game GAME MENU inside the engine's process (the game menu's
//! design, section 2). Its logic is `crate::gamemenu`; this is the driver
//! on the window thread, which ticks at least every 5 ms. Each tick it:
//!
//! 1. reads player 0's pad itself (the pad the engine's polls picked),
//!    the input script's pad buttons, stick and keys, and the menu keys
//!    our window got (Esc, the arrows, Enter, Backspace);
//! 2. turns them into menu keys (`gamemenu::edges`) and runs `GameMenu`;
//! 3. applies what it says: the input gate opened or closed
//!    (`input::open_menu`, `input::close_menu`), the live controls, the
//!    lobby's events, the quit, and once it closes the engine's profile
//!    and gamepad mapping for the controls it left, with the reload
//!    message that makes the engine read them again;
//! 4. draws the panel on the CPU when the page or the back buffer's size
//!    changed, and publishes it for the render thread (`overlay`);
//! 5. checks that the panel was drawn: one not drawn within a second
//!    (window not minimised) closes the menu and lets go of the game's
//!    input, and three of those, or any overlay failure, turn the menu
//!    off for the run, giving Start back to the engine and telling the
//!    lobby (`H2EVENT menu off`). PLEASE WAIT is exempt: it stays until
//!    the process ends. Those rules are `gamemenu::watch`'s;
//! 6. saves the controls changed, once the player leaves CONTROLLER
//!    SETTINGS or the menu, a second after the last change, or before a
//!    quit (`gamemenu::save`).
//!
//! It opens with Start on player 0's pad or Esc in the game's window,
//! once the fonts are loaded (`prepare`, at map load), while the game is
//! on and no quit has started. The match keeps running; the engine just
//! gets neutral input for player 0.
//!
//! Nothing here touches halo2.dll beyond the host interface: engine
//! messages through `engine::post_message`, the window thread being its
//! caller, and answers to the engine's own calls.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, IsIconic, ASFW_ANY};

use super::log::log;
use super::{engine, gfx, input, overlay, Setup};
use crate::controls::Controls;
use crate::gamemenu::edges::PadEdges;
use crate::gamemenu::save::{Saves, Unsaved};
use crate::gamemenu::watch::{self, Act, Off, Watch, END_GAME_WAIT, HEALTH_CLOSES, HEALTH_WAIT};
use crate::gamemenu::{self, Effect, GameMenu, GameMenuFrame, Key, Leave, MenuContext, Pickup};
use crate::live::{self, Engine};
use crate::profile::{pad, InputFrame};

/// Whether the engine picks up a new player profile mid-match (look
/// sensitivity, automatic look centering) by itself. PC check 7 found it
/// doesn't: the call counts of slot 34 stayed flat. The reload message
/// (`--reload-message`, 5 by default) makes it (check 8); with that off,
/// the menu tags those settings NEXT GAME once changed.
pub const ENGINE_REREADS_PROFILE: bool = false;
/// Whether the engine picks up a new gamepad mapping mid-match (the
/// button layout) by itself: no (check 7, slot 116), as above.
pub const ENGINE_REREADS_MAPPING: bool = false;

/// The fonts are loaded and the menu may open; Start and Esc are its own.
static READY: AtomicBool = AtomicBool::new(false);
/// `prepare` has run.
static PREPARED: AtomicBool = AtomicBool::new(false);
static FONTS: OnceLock<h2ui::text::Fonts> = OnceLock::new();

pub fn ready() -> bool {
    READY.load(Ordering::SeqCst)
}

pub fn is_open() -> bool {
    input::menu_open()
}

/// At map load (host slot 3, state 1): loads Halo 2's fonts from MCC's
/// `halo2\h2_fonts` (or uses the built-in font) on a thread of its own,
/// then makes the menu ready, unless `--no-game-menu` or
/// `H2LAUNCH_OVERLAY_FAIL=1` keep it off (`stall` and `copy` make the
/// overlay fail later instead: `overlay::set_stand_in`). Runs once.
pub fn prepare() {
    if PREPARED.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(s) = super::setup() else { return };
    if !s.args.game_menu {
        log!("game menu: off (--no-game-menu): Start and Esc reach the game");
        return;
    }
    let stand_in = std::env::var("H2LAUNCH_OVERLAY_FAIL").unwrap_or_default();
    if stand_in == "1" {
        log!("game menu: off (H2LAUNCH_OVERLAY_FAIL=1): Start and Esc reach the game");
        return;
    }
    if overlay::set_stand_in(&stand_in) {
        log!("game menu: H2LAUNCH_OVERLAY_FAIL={stand_in}: the overlay will fail once the menu shows (a stand-in for the safety checks)");
    }
    let dir = crate::mccroot::join(&s.root, r"halo2\h2_fonts");
    let spawned = std::thread::Builder::new()
        .name("h2launch-menu-fonts".into())
        .spawn(move || {
            let start = Instant::now();
            let fonts = h2ui::text::Fonts::read(Path::new(&dir));
            let loaded = fonts.loaded();
            let which = if loaded.is_empty() {
                format!("the built-in font; none read from {dir}")
            } else {
                format!("Halo 2's from {dir}: {}", loaded.join(", "))
            };
            let _ = FONTS.set(fonts);
            READY.store(true, Ordering::SeqCst);
            log!(
                "game menu: ready (fonts: {which}; {:.0} ms); Start on the pad or Esc in the window opens it",
                start.elapsed().as_secs_f64() * 1000.0
            );
        });
    if let Err(e) = spawned {
        log!("game menu: off (no thread for its fonts: {e})");
        // The lobby said the menu opens with Start; it won't.
        live::tell(Engine::MenuOff);
    }
}

/// A key for the menu: from the pad (and the script's pad), or a keyboard
/// key by its virtual-key code (our window's, or the script's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Press {
    Pad(Key),
    Keyboard(u32),
}

/// The menu on the window thread.
pub struct Driver {
    menu: GameMenu,
    ctx: MenuContext,
    edges: PadEdges,
    /// The XInput slot read last (-1: none; None: nothing read yet).
    pad: Option<i32>,
    /// The input script's time at the last tick (this driver's own, apart
    /// from the input polls').
    script_t: Option<f64>,
    /// The script's keys held at the last tick.
    script_keys: Vec<u8>,
    /// The frame and the back buffer size of the panel published last.
    shown: Option<(GameMenuFrame, (u32, u32))>,
    /// The overlay generation of that panel.
    published: u64,
    /// The safety rules: the panel's health, turning off, the game's end
    /// and End Game's wait.
    watch: Watch,
    /// The controls the engine's profile and gamepad mapping were last
    /// built from (`host::publish_controls`).
    engine_controls: Controls,
    /// The controls changed in the menu and not saved yet.
    saves: Saves,
}

impl Driver {
    pub fn new(s: &Setup) -> Driver {
        let players = match &s.session {
            Some((sess, me)) => {
                gamemenu::roster(sess, *me, super::VARIANT_TEAMS.load(Ordering::SeqCst))
            }
            None => gamemenu::offline_roster(&s.args.name),
        };
        let reload = s.args.reload_message.is_some();
        let ctx = MenuContext {
            kind: s.args.game_kind,
            host: s
                .session
                .as_ref()
                .is_none_or(|(sess, me)| sess.is_host(*me)),
            launch: s.controls,
            profile: Pickup::of(ENGINE_REREADS_PROFILE, reload),
            mapping: Pickup::of(ENGINE_REREADS_MAPPING, reload),
            players,
        };
        Driver {
            menu: GameMenu::new(s.controls),
            ctx,
            edges: PadEdges::new(),
            pad: None,
            script_t: None,
            script_keys: Vec::new(),
            shown: None,
            published: 0,
            watch: Watch::new(),
            engine_controls: s.controls,
            saves: Saves::new(),
        }
    }

    /// One pass of the window thread's loop (`t`: seconds since start):
    /// the menu, then the controls it changed saved if it is time.
    pub fn tick(&mut self, t: f64) {
        self.run(t);
        if let Some(u) = self.saves.due(t, self.menu.page()) {
            self.save(u);
        }
    }

    fn run(&mut self, t: f64) {
        let off = self.watch.is_off();
        if !off && !ready() {
            // Presses before the menu is ready never open it later.
            let _ = input::take_menu_keys();
            return;
        }
        // Turned off, only End Game's wait goes on (a confirmed End Game
        // still quits on time).
        let (reading, presses) = if off {
            Default::default()
        } else {
            self.read(t)
        };
        let ended = super::GAME_ENDED.load(Ordering::SeqCst);
        let acts = self
            .watch
            .start(t, overlay::failed(), self.menu.page(), ended);
        self.act(acts, &reading, t);
        if self.watch.is_off() {
            return;
        }
        for p in presses {
            let fx = if self.menu.is_open() {
                let before = self.menu.page();
                let fx = match p {
                    Press::Pad(k) => self.menu.press(k, false, &self.ctx),
                    Press::Keyboard(code) => match gamemenu::menu_key(code) {
                        Some(k) => self.menu.press(k, true, &self.ctx),
                        None => Vec::new(),
                    },
                };
                if self.menu.page() != before {
                    log!("game menu: {p:?}: {:?}", self.menu.page());
                }
                fx
            } else if p == Press::Pad(Key::Start) || p == Press::Keyboard(gamemenu::vk::ESCAPE) {
                if self.can_open() {
                    let fx = self.menu.open();
                    // A stick or d-pad direction held in the game moves
                    // nothing here until it is let go.
                    self.edges.hold_directions();
                    self.menu.set_keyboard(matches!(p, Press::Keyboard(_)));
                    log!(
                        "game menu: opened ({})",
                        if matches!(p, Press::Keyboard(_)) {
                            "Esc"
                        } else {
                            "Start"
                        }
                    );
                    fx
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            self.apply(fx, &reading, t);
        }
        if self.menu.is_open() {
            // A failed overlay draws nothing more (it shows nothing, and
            // a failed publish isn't tried again every tick).
            if !overlay::failed() {
                self.draw();
            }
            let failed = overlay::failed();
            if !failed {
                overlay::show(true);
            }
            // SAFETY: a query on our own window.
            let iconic = unsafe { IsIconic(super::hwnd()) }.as_bool();
            let behind = watch::behind(self.shown.is_some(), self.published, overlay::drawn());
            let acts = self
                .watch
                .panel(t, failed, self.menu.page(), behind, iconic);
            self.act(acts, &reading, t);
        }
    }

    /// Does what the safety rules say (`gamemenu::watch`).
    fn act(&mut self, acts: Vec<Act>, reading: &InputFrame, t: f64) {
        for a in acts {
            match a {
                Act::NotDrawn(n) => {
                    log!(
                        "game menu: its panel wasn't drawn for {HEALTH_WAIT} s; closed it and let go of the game's input ({n} of {HEALTH_CLOSES})"
                    );
                    self.force_close(reading);
                }
                Act::EndGameEnded => {
                    // The End Game experiment ended the game: show its end.
                    log!("game menu: the game ended after End Game's engine message");
                    self.force_close(reading);
                }
                Act::GameOver => {
                    let fx = self.menu.game_over();
                    if !fx.is_empty() {
                        log!("game menu: closed (game over)");
                    }
                    self.apply(fx, reading, t);
                }
                Act::TurnOff(why) => self.turn_off(
                    match why {
                        Off::Failed => "the overlay failed",
                        Off::NotDrawn => "its panel didn't show three times",
                    },
                    reading,
                ),
                Act::Quit => {
                    log!("game menu: the game didn't end {END_GAME_WAIT} s after End Game's engine message; quitting");
                    super::set_reason("ended from the game menu");
                    super::request_quit(super::QUIT_USER);
                }
            }
        }
    }

    /// A quit started (from the menu or not): PLEASE WAIT if it is open,
    /// and the controls changed in it saved now.
    pub fn quitting(&mut self) {
        self.menu.quitting();
        self.save_now();
    }

    /// Saves the controls changed and not saved yet, if any.
    fn save_now(&mut self) {
        if let Some(u) = self.saves.take() {
            self.save(u);
        }
    }

    /// The menu may open: ready, the map loaded, the game on, no quit.
    fn can_open(&self) -> bool {
        !self.watch.is_off()
            && ready()
            && super::marked(&super::STATE1_AT).is_some()
            && !super::GAME_ENDED.load(Ordering::SeqCst)
            && super::QUIT.load(Ordering::SeqCst) == super::QUIT_NONE
    }

    /// One reading of everything the menu listens to: what is held (for
    /// the hold when it closes) and the presses since the last tick.
    fn read(&mut self, t: f64) -> (InputFrame, Vec<Press>) {
        let mut r = InputFrame::default();
        let pad = input::menu_pad();
        let index = pad.map_or(-1, |(i, _)| i);
        if self.pad != Some(index) {
            // Another pad (or none): what it holds now counts once let go,
            // except Start with the menu closed. The engine's poll picks a
            // pad when it is used (padpick), so its Start may be the very
            // press that picked it, and that press is the menu's (the gate
            // took it from the engine). The first reading, when the menu
            // gets ready, settles everything: a Start held through the
            // load is old.
            if self.pad.is_none() || self.menu.is_open() {
                self.edges.reset();
            } else {
                self.edges.reset_except(pad::START);
            }
            self.pad = Some(index);
        }
        let (mut lx, mut ly) = (0i16, 0i16);
        if let Some((_, g)) = pad {
            r.pad_buttons = g.buttons;
            r.left_trigger = g.left_trigger;
            r.right_trigger = g.right_trigger;
            (lx, ly) = (g.lx, g.ly);
        }
        let mut script_keys = Vec::new();
        if let (Some(sc), Some(st)) = (
            super::setup().and_then(|s| s.script.as_ref()),
            super::script_time(),
        ) {
            let t0 = self.script_t.replace(st).unwrap_or(st);
            let syn = sc.sample(t0.min(st), st);
            r.pad_buttons |= syn.pad_buttons;
            r.left_trigger = r.left_trigger.max(syn.left_trigger);
            r.right_trigger = r.right_trigger.max(syn.right_trigger);
            r.mouse_buttons |= syn.mouse_buttons;
            if let Some((x, y)) = syn.left_stick {
                (lx, ly) = (x, y);
            }
            for &k in &syn.keys {
                r.keys[k as usize] = true;
            }
            script_keys = syn.keys;
        }
        input::add_held_keys(&mut r);
        // The left stick read raw, before the thumbstick layout.
        let mut presses: Vec<Press> = self
            .edges
            .update(r.pad_buttons, lx, ly, t)
            .into_iter()
            .map(Press::Pad)
            .collect();
        for &k in &script_keys {
            let code = u32::from(k);
            let menu = gamemenu::menu_key(code).is_some();
            if menu && !self.script_keys.contains(&k) {
                presses.push(Press::Keyboard(code));
            }
        }
        self.script_keys = script_keys;
        presses.extend(input::take_menu_keys().into_iter().map(Press::Keyboard));
        (r, presses)
    }

    fn apply(&mut self, fx: Vec<Effect>, reading: &InputFrame, t: f64) {
        for e in fx {
            match e {
                Effect::Opened => input::open_menu(),
                Effect::Closed => {
                    self.closed(reading);
                    log!("game menu: closed");
                }
                Effect::Controls(c) => self.controls(c, false, t),
                Effect::Defaults => self.controls(Controls::default(), true, t),
                Effect::Leave(k) => self.leave(k, t),
            }
        }
    }

    /// The menu closed: the input gate takes what is held, the panel
    /// goes, and the engine gets the controls it left.
    fn closed(&mut self, reading: &InputFrame) {
        input::close_menu(reading);
        overlay::show(false);
        overlay::clear();
        self.shown = None;
        self.watch.closed();
        self.publish();
    }

    /// A profile and gamepad mapping for the controls the menu left, built
    /// when it closes: one generation a visit rather than one a step (a
    /// held arrow on Mouse Sensitivity would use the cap up in seconds).
    /// The engine gets neutral input while the menu is open, so it misses
    /// nothing by the wait. Then, after a change to a setting the engine
    /// reads only from them, the reload message (`--reload-message`, 5 by
    /// default) makes it read them again (PC check 8); not once the game
    /// has ended or a quit has started, as the next game loads them anyway.
    fn publish(&mut self) {
        let c = self.menu.live();
        let old = std::mem::replace(&mut self.engine_controls, c);
        if c == old {
            return;
        }
        let now = super::host::publish_controls(&c);
        // Past a kind's cap the engine keeps an older one: its settings
        // are the next game's from here on, whatever it re-reads.
        if !now.profile && self.ctx.profile.mid_match() {
            self.ctx.profile = Pickup::NextGame;
            log!("game menu: profile settings take effect in the next game from now on");
        }
        if !now.mapping && self.ctx.mapping.mid_match() {
            self.ctx.mapping = Pickup::NextGame;
            log!("game menu: the button layout takes effect in the next game from now on");
        }
        let load_only = old.buttons != c.buttons
            || old.look_sensitivity != c.look_sensitivity
            || old.auto_center != c.auto_center
            || (!old.vibration && c.vibration);
        let game_on = !super::GAME_ENDED.load(Ordering::SeqCst)
            && super::QUIT.load(Ordering::SeqCst) == super::QUIT_NONE;
        let reload = super::setup().and_then(|s| s.args.reload_message);
        if let (true, true, true, Some(n)) =
            (load_only, game_on, now.profile || now.mapping, reload)
        {
            engine::post_message(n, "--reload-message: the profile or mapping changed");
        }
    }

    /// Closes the menu from any page (keeping the live controls).
    fn force_close(&mut self, reading: &InputFrame) {
        if self.menu.is_open() {
            self.menu = GameMenu::new(self.menu.live());
            self.closed(reading);
        }
    }

    /// The safety rules turned the menu off for the run (`Watch` keeps
    /// that, and a pending End Game still quits): closed, Start and Esc
    /// given back to the engine, and the lobby told, so its in-game
    /// screen stops saying Start opens the menu.
    fn turn_off(&mut self, why: &str, reading: &InputFrame) {
        self.force_close(reading);
        READY.store(false, Ordering::SeqCst);
        overlay::show(false);
        overlay::clear();
        log!("game menu: off for this run ({why}); Start and Esc reach the game again");
        live::tell(Engine::MenuOff);
    }

    /// Draws and publishes the panel if its page or the back buffer's
    /// size changed. The size is the swap chain's: only our
    /// `resize_swap_chain`, on this thread, changes it, while the engine
    /// waits in slot 2.
    fn draw(&mut self) {
        let frame = self.menu.frame(&self.ctx);
        let size = gfx::swap_size();
        if self
            .shown
            .as_ref()
            .is_some_and(|(f, sz)| *f == frame && *sz == size)
        {
            return;
        }
        let Some(fonts) = FONTS.get() else { return };
        let (w, h) = size;
        if w == 0 || h == 0 {
            return;
        }
        let start = Instant::now();
        let panel = h2ui::gamemenu::draw_panel(&frame, fonts, w, h);
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        if panel.w == 0 || panel.h == 0 {
            // A window too small for it; the health check closes the menu.
            return;
        }
        let Some(device) = gfx::device() else { return };
        match overlay::publish(&device, &panel, size) {
            Ok(gen) => {
                log!(
                    "game menu: panel {}x{} at {},{} drawn in {ms:.1} ms for a {w}x{h} back buffer",
                    panel.w,
                    panel.h,
                    panel.x,
                    panel.y
                );
                self.published = gen;
                self.shown = Some((frame, size));
            }
            Err(e) => {
                // Off for the run, as a failed copy is: retried every tick
                // it would redraw the panel and log each 5 ms, while the
                // render thread copied an older page.
                log!("game menu: the panel wasn't published: {e}");
                overlay::fail(&format!("the panel wasn't published ({e})"));
                self.shown = None;
            }
        }
    }

    /// The menu changed the controls at `t`: the launcher's own use of
    /// them now; the engine's profile and mapping follow when the menu
    /// closes (`publish`), and the saving once the player is done with
    /// CONTROLLER SETTINGS (`gamemenu::save`). `defaults`: Use Default
    /// Settings, which saves every one (the lobby is told so).
    fn controls(&mut self, c: Controls, defaults: bool, t: f64) {
        let old = super::controls();
        super::set_controls(c);
        if old.vibration && !c.vibration {
            input::stop_rumble();
            log!("game menu: rumble stopped (vibration off)");
        }
        log!("game menu: controls now {}", c.describe());
        self.saves.change(old, c, defaults, t);
    }

    /// Saves the controls the menu changed: the lobby is told (it saves
    /// them; lobby.txt is its file in a match), or with no lobby they go
    /// into lobby.txt here.
    fn save(&mut self, u: Unsaved) {
        let event = Engine::controls(u.to, u.defaults);
        if live::to_lobby() {
            live::tell(event);
        } else {
            log!("game menu: {}", live::event_line(&event));
            save_controls(&u.from, &u.to, u.defaults);
        }
    }

    /// Leave Game or End Game confirmed: save the controls changed, tell
    /// the lobby, quit the engine as closing its window does (the menu
    /// shows PLEASE WAIT). End Game's result, which the engine gives as it
    /// quits, then goes to the lobby as a game cut short
    /// (`host::game_result`).
    fn leave(&mut self, k: Leave, t: f64) {
        self.save_now();
        if k == Leave::End {
            super::ENDED_FROM_MENU.store(true, Ordering::SeqCst);
        }
        let event = Engine::Left(k);
        if !live::to_lobby() {
            log!("game menu: {}", live::event_line(&event));
        }
        live::tell(event);
        // The lobby may take the foreground back once this process ends.
        // SAFETY: a plain call with a constant.
        if let Err(e) = unsafe { AllowSetForegroundWindow(ASFW_ANY) } {
            log!("game menu: AllowSetForegroundWindow: {e}");
        }
        let reason = match k {
            Leave::Game => "left from the game menu",
            Leave::End => "ended from the game menu",
        };
        let experiment = super::setup()
            .and_then(|s| s.args.end_game_message)
            .filter(|_| k == Leave::End);
        match experiment {
            Some(n) => {
                log!("game menu: End Game: posting engine message {n} (--end-game-message); quitting in {END_GAME_WAIT} s unless the game ends");
                engine::post_message(n, "--end-game-message");
                self.watch.end_game(t);
            }
            None => {
                log!("game menu: {reason}");
                super::set_reason(reason);
                super::request_quit(super::QUIT_USER);
            }
        }
    }
}

/// Saves the change from `old` to `c` in `lobby.txt` in the launcher's
/// folder (with `--instance`, its own), for runs with no lobby
/// (`--offline`, `--live`): only the settings the player changed, so a
/// control flag given for this run isn't saved with them, or every one
/// for Use Default Settings (`defaults`; `Controls::with_changes`).
fn save_controls(old: &Controls, c: &Controls, defaults: bool) {
    let dir = crate::lobby::settings::folder(super::instance());
    let path = dir.join("lobby.txt");
    let mut s = crate::lobby::settings::Settings::load(&path);
    s.controls = s.controls.with_changes(old, c, defaults);
    let saved = std::fs::create_dir_all(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))
        .and_then(|_| s.save(&path));
    match saved {
        Ok(()) => log!("game menu: controls saved in {}", path.display()),
        Err(e) => log!("game menu: the controls weren't saved: {e}"),
    }
}
