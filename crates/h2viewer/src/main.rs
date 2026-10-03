//! h2viewer: fly around a Halo 2 level.
//!
//! Usage: h2viewer [path\to\level.map]
//! With no argument it looks for lockout.map in the usual install folders.
//!
//! Controls: click to capture the mouse, WASD move, Space/C up/down,
//! Shift fast, Esc releases the mouse (Esc again quits).

mod camera;
mod gpu;

use blam_cache::{geometry::Mesh, CacheFile, PlayerSpawn};
use camera::FlyCamera;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

const DEFAULT_MAP_DIRS: &[&str] = &[
    r"C:\Games\Halo 2 Project Cartographer\maps",
    r"C:\Program Files (x86)\Microsoft Games\Halo 2\maps",
    r"C:\Program Files\Microsoft Games\Halo 2\maps",
    "maps",
];

fn find_map() -> Result<PathBuf, String> {
    if let Some(arg) = std::env::args_os().nth(1) {
        return Ok(PathBuf::from(arg));
    }
    DEFAULT_MAP_DIRS
        .iter()
        .map(|d| PathBuf::from(d).join("lockout.map"))
        .find(|p| p.exists())
        .ok_or_else(|| {
            "Couldn't find lockout.map. Drag a .map file onto h2viewer.exe, or run: h2viewer <path to .map>".into()
        })
}

/// Spartan eye height above a spawn point, in world units (1 unit = 10 ft).
const EYE_HEIGHT: f32 = 0.62;

fn load_level(path: &PathBuf) -> Result<(Mesh, Option<PlayerSpawn>), Box<dyn std::error::Error>> {
    let mut map = CacheFile::open(path)?;
    let mut mesh = Mesh::default();
    for bsp in map.structure_bsps()? {
        mesh.append(&map.bsp_collision_mesh(&bsp)?);
    }
    if mesh.indices.is_empty() {
        return Err("map has no level geometry".into());
    }
    let spawn = map.player_spawns().ok().and_then(|s| s.first().copied());
    Ok((mesh, spawn))
}

/// Center of the densest part of the level, used as the spawn point.
fn level_focus(mesh: &Mesh) -> (glam::Vec3, f32) {
    let mut lo = [0f32; 3];
    let mut hi = [0f32; 3];
    for k in 0..3 {
        let mut v: Vec<f32> = mesh.positions.iter().map(|p| p[k]).collect();
        v.sort_by(f32::total_cmp);
        lo[k] = v[v.len() / 10];
        hi[k] = v[v.len() * 9 / 10];
    }
    let lo = glam::Vec3::from(lo);
    let hi = glam::Vec3::from(hi);
    ((lo + hi) * 0.5, (hi - lo).length() * 0.5)
}

struct App {
    mesh: Mesh,
    title: String,
    window: Option<Arc<Window>>,
    gpu: Option<gpu::Gpu>,
    camera: FlyCamera,
    keys: HashSet<KeyCode>,
    captured: bool,
    last_frame: Instant,
}

impl App {
    fn set_capture(&mut self, on: bool) {
        let Some(w) = &self.window else { return };
        if on {
            let grabbed = w
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| w.set_cursor_grab(CursorGrabMode::Confined));
            if grabbed.is_err() {
                return;
            }
        } else {
            let _ = w.set_cursor_grab(CursorGrabMode::None);
        }
        w.set_cursor_visible(!on);
        self.captured = on;
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(&self.title)
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        match pollster::block_on(gpu::Gpu::new(window.clone(), &self.mesh)) {
            Ok(g) => self.gpu = Some(g),
            Err(e) => {
                eprintln!("graphics init failed: {e}");
                event_loop.exit();
                return;
            }
        }
        self.window = Some(window);
        self.last_frame = Instant::now();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(g) = &mut self.gpu {
                    g.resize(size.width, size.height);
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                self.set_capture(true);
            }
            WindowEvent::Focused(false) => {
                self.set_capture(false);
                self.keys.clear();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                match event.state {
                    ElementState::Pressed => {
                        if code == KeyCode::Escape && !event.repeat {
                            if self.captured {
                                self.set_capture(false);
                            } else {
                                event_loop.exit();
                            }
                        }
                        self.keys.insert(code);
                    }
                    ElementState::Released => {
                        self.keys.remove(&code);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = (now - self.last_frame).as_secs_f32().min(0.1);
                self.last_frame = now;
                self.camera.update(&self.keys, dt);
                if let Some(g) = &mut self.gpu {
                    g.render(self.camera.view_proj(g.aspect()), self.camera.position);
                }
            }
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.captured {
                self.camera.look(delta.0 as f32, delta.1 as f32);
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        // Keep the console open when launched by double-click.
        eprintln!("Press Enter to close.");
        let _ = std::io::stdin().read_line(&mut String::new());
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let path = find_map()?;
    println!("loading {}", path.display());
    let (mesh, spawn) = load_level(&path)?;
    println!("{} triangles", mesh.triangle_count());
    let camera = match spawn {
        Some(s) => {
            let eye = glam::Vec3::from(s.position) + glam::Vec3::Z * EYE_HEIGHT;
            FlyCamera::looking_at(eye, eye + glam::vec3(s.facing.cos(), s.facing.sin(), 0.0))
        }
        None => {
            let (focus, radius) = level_focus(&mesh);
            FlyCamera::looking_at(
                focus + glam::vec3(radius * 0.6, -radius * 0.6, radius * 0.4),
                focus,
            )
        }
    };
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        mesh,
        title: format!("Halo 2 Rust: {name} (click to look, WASD to fly, Esc to release)"),
        window: None,
        gpu: None,
        camera,
        keys: HashSet::new(),
        captured: false,
        last_frame: Instant::now(),
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}
