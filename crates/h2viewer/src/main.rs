//! h2viewer: fly around a Halo 2 level.
//!
//! Usage: h2viewer [path\to\level.map]
//! With no argument it looks for lockout.map in the usual install folders.
//!
//! Controls: click to capture the mouse, WASD move, Space jump, Ctrl/C crouch,
//! Tab toggles walking / flying (fly: Space/C up/down, Shift fast),
//! Esc releases the mouse (Esc again quits).

mod camera;
mod gpu;
mod scene;

use blam_cache::geometry::Mesh;
use camera::FlyCamera;
use h2sim::{Input, Player, World};
use scene::Scene;
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
    scene: Scene,
    world: World,
    player: Player,
    walking: bool,
    spawn_point: glam::Vec3,
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
        match pollster::block_on(gpu::Gpu::new(window.clone(), &self.scene)) {
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
                        if code == KeyCode::Tab && !event.repeat {
                            self.walking = !self.walking;
                            if self.walking {
                                // Drop in where the fly camera is.
                                let feet = self.camera.position
                                    - glam::Vec3::Z * self.player.biped.standing_camera_height;
                                self.player.position = feet;
                                self.player.velocity = glam::Vec3::ZERO;
                            }
                        }
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
                if self.walking {
                    let held = |k: KeyCode| self.keys.contains(&k);
                    let axis = |pos: KeyCode, neg: KeyCode| {
                        held(pos) as i32 as f32 - held(neg) as i32 as f32
                    };
                    let input = Input {
                        movement: glam::vec2(
                            axis(KeyCode::KeyD, KeyCode::KeyA),
                            axis(KeyCode::KeyW, KeyCode::KeyS),
                        ),
                        yaw: self.camera.yaw,
                        jump: held(KeyCode::Space),
                        crouch: held(KeyCode::ControlLeft) || held(KeyCode::KeyC),
                    };
                    self.player.update(&self.world, input, dt);
                    // Fell out of the level: back to the spawn.
                    if self.player.position.z < self.world.min.z - 1.0 {
                        self.player.position = self.spawn_point;
                        self.player.velocity = glam::Vec3::ZERO;
                    }
                    self.camera.position = self.player.eye();
                } else {
                    self.camera.update(&self.keys, dt);
                }
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
    let scene = Scene::load(&path)?;
    println!(
        "{} triangles, {} textures",
        scene.triangle_count(),
        scene.textures.len() - 1
    );
    let camera = match scene.spawn {
        Some(s) => {
            let eye =
                glam::Vec3::from(s.position) + glam::Vec3::Z * scene.biped.standing_camera_height;
            FlyCamera::looking_at(eye, eye + glam::vec3(s.facing.cos(), s.facing.sin(), 0.0))
        }
        None => {
            let (focus, radius) = level_focus(&scene.collision);
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

    let world = World::new(&scene.collision.positions, &scene.collision.indices);
    let walking = scene.spawn.is_some();
    let spawn_point =
        camera.position - glam::Vec3::Z * scene.biped.standing_camera_height + glam::Vec3::Z * 0.05;
    let player = Player::new(spawn_point, scene.movement, scene.biped);

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        scene,
        world,
        player,
        walking,
        spawn_point,
        title: format!("Halo 2 Rust: {name} (click to look, WASD move, Space jump, Ctrl crouch, Tab walk/fly, Esc release)"),
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
