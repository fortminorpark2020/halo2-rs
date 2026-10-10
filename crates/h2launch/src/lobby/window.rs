//! The lobby's window: winit for the window, keyboard and mouse, softbuffer
//! to show the picture `App` draws, and gilrs for Xbox controllers (the
//! bumpers are LB and RB, as are Page Up and Page Down on the keyboard).

use super::app::{App, Input};
use super::canvas::{Canvas, Text};
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

/// How often the lobby wakes to poll the server and the engine and redraw.
const FRAME: Duration = Duration::from_millis(33);
/// How far a stick goes before it counts as a press.
const STICK: f32 = 0.6;

type Surface = softbuffer::Surface<Rc<Window>, Rc<Window>>;

struct Shell {
    app: App,
    text: Text,
    canvas: Canvas,
    window: Option<Rc<Window>>,
    surface: Option<Surface>,
    pads: Option<Gilrs>,
    /// Which way the left stick is held (-1, 0, 1), so holding it moves
    /// once.
    stick: i32,
    /// The window has the focus. A controller is read whichever window is
    /// in front, so while the engine's is, the lobby leaves it alone.
    focused: bool,
    mouse: (f32, f32),
    error: Option<String>,
}

/// Open the window and run the lobby until it closes.
pub fn run(app: App, text: Text) -> Result<(), String> {
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
        text,
        canvas: Canvas::new(1, 1),
        window: None,
        surface: None,
        pads,
        stick: 0,
        focused: true,
        mouse: (0.0, 0.0),
        error: None,
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

    fn pads(&mut self) {
        let Some(g) = &mut self.pads else {
            return;
        };
        let mut inputs = Vec::new();
        while let Some(ev) = g.next_event() {
            match ev.event {
                EventType::ButtonPressed(b, _) => {
                    let i = match b {
                        Button::South | Button::Start => Some(Input::A),
                        Button::East | Button::Select => Some(Input::B),
                        Button::West => Some(Input::X),
                        Button::North => Some(Input::Y),
                        // gilrs calls the bumpers the triggers.
                        Button::LeftTrigger => Some(Input::Lb),
                        Button::RightTrigger => Some(Input::Rb),
                        Button::DPadUp => Some(Input::Up),
                        Button::DPadDown => Some(Input::Down),
                        Button::DPadLeft => Some(Input::Left),
                        Button::DPadRight => Some(Input::Right),
                        _ => None,
                    };
                    inputs.extend(i);
                }
                EventType::AxisChanged(Axis::LeftStickY, v, _) => {
                    // Up is positive.
                    let held = if v > STICK {
                        1
                    } else if v < -STICK {
                        -1
                    } else {
                        0
                    };
                    if held != self.stick && held != 0 {
                        inputs.push(if held > 0 { Input::Up } else { Input::Down });
                    }
                    self.stick = held;
                }
                _ => {}
            }
        }
        if !self.focused {
            return;
        }
        for i in inputs {
            self.app.input(i);
        }
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
            WindowEvent::Focused(on) => self.focused = on,
            WindowEvent::RedrawRequested => {
                if let Err(e) = self.draw() {
                    self.error = Some(format!("drawing failed: {e}"));
                    el.exit();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
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
                match i {
                    Some(i) => self.app.input(i),
                    None => {
                        let typed = event.text.as_deref().unwrap_or("");
                        for c in typed.chars().filter(|c| !c.is_control()) {
                            self.app.input(Input::Char(c));
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
            } => self.app.click(self.mouse.0, self.mouse.1),
            WindowEvent::MouseWheel { delta, .. } => {
                let up = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y > 0.0,
                    MouseScrollDelta::PixelDelta(p) => p.y > 0.0,
                };
                self.app.input(if up { Input::Up } else { Input::Down });
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        self.pads();
        self.app.tick(Instant::now());
        if self.app.quitting() {
            el.exit();
            return;
        }
        if let Some(w) = &self.window {
            if self.app.take_focus() {
                w.focus_window();
                w.request_user_attention(Some(winit::window::UserAttentionType::Informational));
            }
            w.request_redraw();
        }
        el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + FRAME));
    }
}
