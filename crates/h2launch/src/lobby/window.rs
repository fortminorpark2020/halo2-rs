//! The lobby's window: winit for the window, keyboard and mouse, softbuffer
//! to show the picture `App` draws, and gilrs for Xbox controllers (the
//! bumpers are LB and RB, as are Page Up and Page Down on the keyboard;
//! the d-pad and the left stick move).

use super::app::{App, Input};
use super::canvas::{Canvas, Text};
use super::sound::MenuSound;
use gilrs::{Axis, Button, EventType, Gilrs};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

/// How far a stick goes before it counts as a press: Halo 2's menus
/// count it past about 90% (H; menu.md 1.2).
const STICK: f32 = 0.9;
/// A direction held on the pad repeats this long after its press (an
/// estimate: Halo 2's own first delay isn't known) ...
const REPEAT_FIRST: Duration = Duration::from_millis(250);
/// ... and then once each this long, as Halo 2's menus repeat (H).
const REPEAT_EVERY: Duration = Duration::from_millis(250);

type Surface = softbuffer::Surface<Rc<Window>, Rc<Window>>;

struct Shell {
    app: App,
    /// The menus' music and sounds, played as `app` asks.
    sound: MenuSound,
    text: Text,
    canvas: Canvas,
    window: Option<Rc<Window>>,
    surface: Option<Surface>,
    pads: Option<Gilrs>,
    /// The left stick's last values (x, y; up and right positive).
    stick: (f32, f32),
    /// Which way it is held (`stick_way`), so pushing it moves once,
    /// then by `repeat`.
    stick_way: Option<Input>,
    /// The direction held on the d-pad or the stick, repeated while it is.
    repeat: Repeat,
    /// The window has the focus. A controller is read whichever window is
    /// in front, so while the engine's is, the lobby leaves it alone.
    focused: bool,
    mouse: (f32, f32),
    error: Option<String>,
    /// When the next frame is drawn. A redraw wakes the loop again at once,
    /// so asking for one on every wake would draw as fast as the CPU goes.
    /// (The loop wakes between frames drawn less often: `next_wake`.)
    next_frame: Instant,
}

/// Open the window and run the lobby until it closes.
pub fn run(app: App, text: Text, sound: MenuSound) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    let pads = match Gilrs::new() {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("lobby: no controllers ({e})");
            None
        }
    };
    let mut shell = Shell {
        app,
        sound,
        text,
        canvas: Canvas::new(1, 1),
        window: None,
        surface: None,
        pads,
        stick: (0.0, 0.0),
        stick_way: None,
        repeat: Repeat::default(),
        focused: true,
        mouse: (0.0, 0.0),
        error: None,
        next_frame: Instant::now(),
    };
    event_loop.run_app(&mut shell).map_err(|e| e.to_string())?;
    shell.app.shutdown();
    match shell.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

impl Shell {
    fn open(&mut self, el: &ActiveEventLoop) -> Result<(), String> {
        let attrs = Window::default_attributes()
            .with_title("h2launch")
            .with_inner_size(LogicalSize::new(1280.0, 720.0))
            .with_min_inner_size(LogicalSize::new(640.0, 360.0));
        let window = Rc::new(el.create_window(attrs).map_err(|e| e.to_string())?);
        let context = softbuffer::Context::new(window.clone()).map_err(|e| e.to_string())?;
        let surface = Surface::new(&context, window.clone()).map_err(|e| e.to_string())?;
        self.window = Some(window);
        self.surface = Some(surface);
        Ok(())
    }

    fn draw(&mut self) -> Result<(), String> {
        let (Some(window), Some(surface)) = (&self.window, &mut self.surface) else {
            return Ok(());
        };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else {
            // Minimised.
            return Ok(());
        };
        self.canvas.resize(w.get() as usize, h.get() as usize);
        self.app.draw(&mut self.canvas, &mut self.text);
        surface.resize(w, h).map_err(|e| e.to_string())?;
        let mut buffer = surface.buffer_mut().map_err(|e| e.to_string())?;
        buffer.copy_from_slice(&self.canvas.px);
        buffer.present().map_err(|e| e.to_string())
    }

    /// The controllers' presses since the last call, for the lobby (as
    /// `App::pad`, so its legends show the pad's buttons), and the
    /// directions held for `repeat`.
    fn pads(&mut self) {
        let Some(g) = &mut self.pads else {
            return;
        };
        let now = Instant::now();
        let mut inputs = Vec::new();
        while let Some(ev) = g.next_event() {
            match ev.event {
                EventType::ButtonPressed(b, _) => {
                    let i = pad_button(b);
                    if let Some(d) = i.filter(|&i| is_direction(i)) {
                        self.repeat.pressed(d, now);
                    }
                    inputs.extend(i);
                }
                EventType::ButtonReleased(b, _) => {
                    if let Some(d) = pad_button(b).filter(|&i| is_direction(i)) {
                        self.repeat.released(d);
                    }
                }
                EventType::AxisChanged(axis @ (Axis::LeftStickX | Axis::LeftStickY), v, _) => {
                    if axis == Axis::LeftStickY {
                        self.stick.1 = v;
                    } else {
                        self.stick.0 = v;
                    }
                    let way = stick_way(self.stick.0, self.stick.1);
                    if way != self.stick_way {
                        inputs.extend(way);
                        self.repeat.stick(way, now);
                    }
                    self.stick_way = way;
                }
                _ => {}
            }
        }
        // Read whichever window is in front, so they are left alone while
        // the engine's is, and while the engine plays at all; nothing held
        // then repeats afterwards until it is pressed again.
        if !self.pad_is_ours() {
            self.repeat.reset();
            return;
        }
        if !inputs.is_empty() {
            self.next_frame = now;
        }
        for i in inputs {
            self.app.pad(i);
        }
    }

    /// The controllers' presses are for the lobby now (`pads`).
    fn pad_is_ours(&self) -> bool {
        self.focused && self.app.wants_pad()
    }

    /// The held direction's repeat, if one is due (`Repeat`).
    fn repeat_pad(&mut self, now: Instant) {
        if !self.pad_is_ours() {
            self.repeat.reset();
            return;
        }
        if let Some(i) = self.repeat.tick(now) {
            self.next_frame = now;
            self.app.pad(i);
        }
    }
}

/// The lobby's press for a controller's button, if it has one.
fn pad_button(b: Button) -> Option<Input> {
    match b {
        Button::South | Button::Start => Some(Input::A),
        Button::East | Button::Select => Some(Input::B),
        Button::West => Some(Input::X),
        Button::North => Some(Input::Y),
        // gilrs calls the bumpers the triggers, and the triggers (pressed
        // past its threshold) the triggers 2.
        Button::LeftTrigger => Some(Input::Lb),
        Button::RightTrigger => Some(Input::Rb),
        Button::LeftTrigger2 => Some(Input::Lt),
        Button::RightTrigger2 => Some(Input::Rt),
        Button::DPadUp => Some(Input::Up),
        Button::DPadDown => Some(Input::Down),
        Button::DPadLeft => Some(Input::Left),
        Button::DPadRight => Some(Input::Right),
        _ => None,
    }
}

fn is_direction(i: Input) -> bool {
    matches!(i, Input::Up | Input::Down | Input::Left | Input::Right)
}

/// A direction held on the pad, which repeats while it is held: the first
/// time `REPEAT_FIRST` after it was pressed, then every `REPEAT_EVERY`.
/// The direction pressed last (on the d-pad, or the stick's way) is the
/// one that repeats; letting it go stops the repeat, as does `reset`.
/// The press itself is the caller's: this gives only the repeats.
#[derive(Debug, Default)]
struct Repeat {
    /// The d-pad's directions held down.
    dpad: Vec<Input>,
    /// The stick's way (`stick_way`).
    stick: Option<Input>,
    /// The direction repeating, and when its next repeat is due.
    held: Option<(Input, Instant)>,
}

impl Repeat {
    /// The d-pad's direction `i` went down at `now`.
    fn pressed(&mut self, i: Input, now: Instant) {
        if !self.dpad.contains(&i) {
            self.dpad.push(i);
        }
        self.held = Some((i, now + REPEAT_FIRST));
    }

    /// The d-pad's direction `i` came up.
    fn released(&mut self, i: Input) {
        self.dpad.retain(|&d| d != i);
        self.stop_unless_held();
    }

    /// The stick's way changed to `way` at `now`.
    fn stick(&mut self, way: Option<Input>, now: Instant) {
        self.stick = way;
        match way {
            Some(w) => self.held = Some((w, now + REPEAT_FIRST)),
            None => self.stop_unless_held(),
        }
    }

    /// The repeating direction stops if neither the d-pad nor the stick
    /// still holds it.
    fn stop_unless_held(&mut self) {
        if let Some((i, _)) = self.held {
            if !self.dpad.contains(&i) && self.stick != Some(i) {
                self.held = None;
            }
        }
    }

    /// The repeat due at `now`, if one is: one at most, however late the
    /// call (a late wake doesn't press it twice).
    fn tick(&mut self, now: Instant) -> Option<Input> {
        let (i, due) = self.held?;
        if now < due {
            return None;
        }
        let next = due + REPEAT_EVERY;
        self.held = Some((i, if next > now { next } else { now + REPEAT_EVERY }));
        Some(i)
    }

    /// When the next repeat is due, if a direction is held.
    fn next_due(&self) -> Option<Instant> {
        self.held.map(|(_, due)| due)
    }

    /// Forget what is held: the window lost the focus, or the pad isn't
    /// the lobby's.
    fn reset(&mut self) {
        *self = Repeat::default();
    }
}

impl ApplicationHandler for Shell {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        if let Err(e) = self.open(el) {
            self.error = Some(format!("the window couldn't open: {e}"));
            el.exit();
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Focused(on) => {
                self.focused = on;
                // Frames come at their own pace again (`draw_interval`).
                self.next_frame = Instant::now();
                if !on {
                    self.repeat.reset();
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.draw() {
                    self.error = Some(format!("drawing failed: {e}"));
                    el.exit();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.next_frame = Instant::now();
                let i = match &event.logical_key {
                    Key::Named(NamedKey::Enter) => Some(Input::A),
                    Key::Named(NamedKey::Escape) => Some(Input::B),
                    Key::Named(NamedKey::Tab) => Some(Input::Tab),
                    Key::Named(NamedKey::Backspace) => Some(Input::Backspace),
                    Key::Named(NamedKey::ArrowUp) => Some(Input::Up),
                    Key::Named(NamedKey::ArrowDown) => Some(Input::Down),
                    Key::Named(NamedKey::ArrowLeft) => Some(Input::Left),
                    Key::Named(NamedKey::ArrowRight) => Some(Input::Right),
                    Key::Named(NamedKey::PageUp) => Some(Input::Lb),
                    Key::Named(NamedKey::PageDown) => Some(Input::Rb),
                    _ => None,
                };
                // Keys go through `key`, which leaves alone the ones meant
                // for the engine's window while it plays (and repeats of a
                // key held down through `key_repeated`).
                let held = event.repeat;
                let mut key = |i| {
                    if held {
                        self.app.key_repeated(i);
                    } else {
                        self.app.key(i);
                    }
                };
                match i {
                    Some(i) => key(i),
                    None => {
                        let typed = event.text.as_deref().unwrap_or("");
                        for c in typed.chars().filter(|c| !c.is_control()) {
                            key(Input::Char(c));
                        }
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = (position.x as f32, position.y as f32);
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                self.next_frame = Instant::now();
                self.app.click(self.mouse.0, self.mouse.1);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let up = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y > 0.0,
                    MouseScrollDelta::PixelDelta(p) => p.y > 0.0,
                };
                self.app.wheel(up);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        self.pads();
        let now = Instant::now();
        self.repeat_pad(now);
        self.app.tick(now);
        let sound = self.app.take_sound();
        let music = self.app.music_on();
        self.sound.update(&sound, music, self.app.engine_running());
        if self.app.quitting() {
            el.exit();
            return;
        }
        let every = self.app.draw_interval(self.focused);
        let poll = self.app.frame_interval();
        let (draw, next, wake) = next_wake(now, self.next_frame, every, poll);
        self.next_frame = next;
        if let Some(w) = &self.window {
            if self.app.take_focus() {
                w.focus_window();
                w.request_user_attention(Some(winit::window::UserAttentionType::Informational));
            }
            if draw {
                w.request_redraw();
            }
        }
        // Woken for the next frame or poll, or sooner for the next repeat.
        let wake = match self.repeat.next_due() {
            Some(due) => due.min(wake),
            None => wake,
        };
        el.set_control_flow(ControlFlow::WaitUntil(wake));
    }
}

/// Whether to draw at `now`, with the next frame due at `next`; when the
/// frame after is due; and when the loop wakes next at the latest. One
/// frame each `every` (the app's, `App::draw_interval`: shorter while the
/// intro movie plays and behind the flythrough with
/// `H2LOBBY_SCENE_FPS=60`, longer behind the engine's window), and none
/// in between however often the loop wakes (a redraw, the mouse moving, a
/// pad). The loop wakes at least each `poll` (`App::frame_interval`), as
/// the lobby polls the server and the engine on each wake, however seldom
/// it draws. A frame due further off than `every` (it just got shorter)
/// comes after `every`.
fn next_wake(
    now: Instant,
    next: Instant,
    every: Duration,
    poll: Duration,
) -> (bool, Instant, Instant) {
    let next = next.min(now + every);
    let (draw, next) = if now >= next {
        (true, now + every)
    } else {
        (false, next)
    };
    (draw, next, next.min(now + poll))
}

/// Which way a left stick at (`x`, `y`) points, up and right positive: the
/// stronger axis only, once it is past `STICK`, so a sloppy diagonal push
/// moves one way and doesn't also change the value on a row.
fn stick_way(x: f32, y: f32) -> Option<Input> {
    if x.abs() > y.abs() {
        if x > STICK {
            Some(Input::Right)
        } else if x < -STICK {
            Some(Input::Left)
        } else {
            None
        }
    } else if y > STICK {
        Some(Input::Up)
    } else if y < -STICK {
        Some(Input::Down)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_come_once_each_frame_however_often_the_loop_wakes() {
        const FRAME: Duration = Duration::from_millis(33);
        let ms = Duration::from_millis;
        let t = Instant::now();
        let (draw, next, wake) = next_wake(t, t, FRAME, FRAME);
        assert!(draw);
        assert_eq!((next, wake), (t + FRAME, t + FRAME));
        // The redraw wakes the loop again straight away: no second frame,
        // and the next wake is the next frame's.
        for k in [0, 1, 10, 32] {
            assert_eq!(
                next_wake(t + ms(k), next, FRAME, FRAME),
                (false, next, next)
            );
        }
        let later = t + FRAME;
        let after = later + FRAME;
        assert_eq!(next_wake(later, next, FRAME, FRAME), (true, after, after));
        // The intro's shorter frames.
        let movie = ms(16);
        let soon = later + movie;
        assert_eq!(next_wake(later, later, movie, movie), (true, soon, soon));
        // Drawn twice a second behind the engine: the loop still wakes
        // each frame's interval, without drawing until the frame is due.
        let slow = ms(500);
        let (draw, next, wake) = next_wake(t, t, slow, FRAME);
        assert!(draw);
        assert_eq!((next, wake), (t + slow, t + FRAME));
        assert_eq!(
            next_wake(t + FRAME, next, slow, FRAME),
            (false, next, t + FRAME + FRAME)
        );
        assert_eq!(
            next_wake(t + ms(490), next, slow, FRAME),
            (false, next, next)
        );
        assert!(next_wake(t + slow, next, slow, FRAME).0);
        // Back to 30 a second (the engine closed): the frame due in half
        // a second comes after one frame's interval.
        let now = t + ms(40);
        assert_eq!(
            next_wake(now, next, FRAME, FRAME),
            (false, now + FRAME, now + FRAME)
        );
    }

    #[test]
    fn a_diagonal_push_moves_one_way() {
        assert_eq!(stick_way(0.0, 0.0), None);
        assert_eq!(stick_way(0.5, 0.5), None);
        assert_eq!(stick_way(0.0, 0.95), Some(Input::Up));
        assert_eq!(stick_way(-0.95, 0.1), Some(Input::Left));
        // Past 60% but short of 90%: nothing yet.
        assert_eq!(stick_way(0.0, 0.8), None);
        assert_eq!(stick_way(-0.9, 0.0), None);
        // Pushed down and a little right, past the threshold: down only.
        assert_eq!(stick_way(0.66, -0.93), Some(Input::Down));
        assert_eq!(stick_way(0.93, -0.66), Some(Input::Right));
        // A full push at 45 degrees on a round gate reaches neither.
        assert_eq!(stick_way(0.707, -0.707), None);
        // The stronger axis short of the threshold: nothing, even if the
        // other is near it.
        assert_eq!(stick_way(0.85, 0.89), None);
    }

    #[test]
    fn a_held_direction_repeats_every_250_ms_after_250() {
        let ms = Duration::from_millis;
        let t = Instant::now();
        let mut r = Repeat::default();
        assert_eq!((r.tick(t), r.next_due()), (None, None));
        r.pressed(Input::Down, t);
        assert_eq!(r.next_due(), Some(t + ms(250)));
        assert_eq!(r.tick(t + ms(249)), None);
        assert_eq!(r.tick(t + ms(250)), Some(Input::Down));
        assert_eq!(r.tick(t + ms(260)), None);
        assert_eq!(r.next_due(), Some(t + ms(500)));
        assert_eq!(r.tick(t + ms(500)), Some(Input::Down));
        // A late wake (the loop slept) repeats once, then counts from it.
        assert_eq!(r.tick(t + ms(1400)), Some(Input::Down));
        assert_eq!(r.tick(t + ms(1400)), None);
        assert_eq!(r.next_due(), Some(t + ms(1650)));
        // Let go: no more.
        r.released(Input::Down);
        assert_eq!((r.tick(t + ms(5000)), r.next_due()), (None, None));
        // The stick: its way repeats, and a new way starts again.
        r.stick(Some(Input::Up), t);
        assert_eq!(r.tick(t + ms(250)), Some(Input::Up));
        r.stick(Some(Input::Left), t + ms(300));
        assert_eq!(r.tick(t + ms(500)), None);
        assert_eq!(r.tick(t + ms(550)), Some(Input::Left));
        r.stick(None, t + ms(600));
        assert_eq!(r.next_due(), None);
        // The newest press repeats; letting go of the other doesn't stop it.
        r.pressed(Input::Up, t);
        r.pressed(Input::Right, t + ms(100));
        r.released(Input::Up);
        assert_eq!(r.tick(t + ms(350)), Some(Input::Right));
        // The stick let go while the d-pad still holds the same way.
        r.stick(Some(Input::Right), t + ms(400));
        r.stick(None, t + ms(450));
        assert_eq!(r.tick(t + ms(650)), Some(Input::Right));
        // Reset (focus lost, or the engine has the pad): nothing held.
        r.reset();
        assert_eq!((r.tick(t + ms(5000)), r.next_due()), (None, None));
        r.released(Input::Right);
        assert_eq!(r.next_due(), None);
    }
}
