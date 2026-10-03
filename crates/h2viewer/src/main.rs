//! h2viewer: walk around a Halo 2 level and shoot its weapons.
//!
//! Usage: h2viewer [path\to\level.map]
//! With no argument it looks for lockout.map in the usual install folders.
//!
//! Controls: click to capture the mouse, WASD move, Space jump, Ctrl/C crouch,
//! left mouse fire, right mouse / Z zoom, R reload, Q / mouse wheel / 1-9
//! switch weapon, Tab toggles walking / flying (fly: Space/C up/down, Shift
//! fast), Esc releases the mouse (Esc again quits).

mod camera;
mod effects;
mod font;
mod gpu;
mod hud;
mod scene;

use blam_cache::geometry::Mesh;
use camera::FlyCamera;
use effects::Effects;
use glam::{Mat4, Vec3};
use gpu::{hud_mode, DrawCall, Frame};
use h2sim::{Input, Player, WeaponInput, WeaponState, World};
use hud::HudBuilder;
use scene::{Scene, WeaponAssets};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

const DEFAULT_MAP_DIRS: &[&str] = &[
    r"C:\Games\Halo 2 Project Cartographer\maps",
    r"C:\Program Files (x86)\Microsoft Games\Halo 2\maps",
    r"C:\Program Files\Microsoft Games\Halo 2\maps",
    "maps",
];

/// Seconds to bring a weapon up after switching to it.
const READY_TIME: f32 = 0.5;

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
fn level_focus(mesh: &Mesh) -> (Vec3, f32) {
    let mut lo = [0f32; 3];
    let mut hi = [0f32; 3];
    for k in 0..3 {
        let mut v: Vec<f32> = mesh.positions.iter().map(|p| p[k]).collect();
        v.sort_by(f32::total_cmp);
        lo[k] = v[v.len() / 10];
        hi[k] = v[v.len() * 9 / 10];
    }
    let lo = Vec3::from(lo);
    let hi = Vec3::from(hi);
    ((lo + hi) * 0.5, (hi - lo).length() * 0.5)
}

/// What a weapon HUD widget shows, from its name in the HUD tag. Dual-wield
/// weapons have `_left` and `_right` versions; held alone they use `_right`.
#[derive(Debug, PartialEq)]
enum HudRole {
    Background,
    AmmoMeter,
    ZoomedAmmoMeter,
    /// Always drawn as is (crosshair, heat and battery frames).
    Static,
    Scope,
    /// Only while zoomed (scope ticks, magnification label).
    Zoomed,
    Hidden,
}

fn hud_role(name: &str, magnification: f32) -> HudRole {
    match name {
        "weapon_background_single" | "weapon_background_right" => HudRole::Background,
        "ammo_meter_single" | "ammo_meter_right" => HudRole::AmmoMeter,
        "ammo_meter_zoomed" => HudRole::ZoomedAmmoMeter,
        "crosshair" | "heat_background" | "heat_background_right" | "battery_meter" => {
            HudRole::Static
        }
        n if n.contains("scope_mask") => HudRole::Scope,
        "left_crosshair" | "right_crosshair" | "top_crosshair" | "bottom_crosshair"
        | "distance_meter" | "covenant_2xa" | "covenant_2xb" => HudRole::Zoomed,
        n => match n.strip_suffix('x').and_then(|m| m.parse::<f32>().ok()) {
            Some(m) if (m - magnification).abs() < 0.5 => HudRole::Zoomed,
            _ => HudRole::Hidden,
        },
    }
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

struct App {
    scene: Scene,
    world: World,
    player: Player,
    walking: bool,
    spawn_point: Vec3,
    title: String,
    window: Option<Arc<Window>>,
    gpu: Option<gpu::Gpu>,
    camera: FlyCamera,
    keys: HashSet<KeyCode>,
    captured: bool,
    last_frame: Instant,
    fire_held: bool,
    zoom_held: bool,
    /// Presses since the last update, so a quick click between frames still counts.
    fire_tapped: bool,
    zoom_tapped: bool,
    weapon: usize,
    weapon_states: Vec<WeaponState>,
    /// Seconds left bringing the current weapon up.
    readying: f32,
    effects: Effects,
    bob_phase: f32,
}

impl App {
    fn set_capture(&mut self, on: bool) {
        let Some(w) = &self.window else { return };
        if on {
            // Some systems refuse to grab; raw mouse motion still works then.
            let _ = w
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| w.set_cursor_grab(CursorGrabMode::Confined));
        } else {
            let _ = w.set_cursor_grab(CursorGrabMode::None);
        }
        w.set_cursor_visible(!on);
        self.captured = on;
    }

    fn current(&self) -> Option<(&WeaponAssets, &WeaponState)> {
        Some((
            self.scene.weapons.get(self.weapon)?,
            self.weapon_states.get(self.weapon)?,
        ))
    }

    fn switch_weapon(&mut self, to: usize) {
        let n = self.scene.weapons.len();
        if n == 0 || to % n == self.weapon {
            return;
        }
        if let Some(s) = self.weapon_states.get_mut(self.weapon) {
            s.zoom = 0;
            s.reloading = None;
        }
        self.weapon = to % n;
        self.readying = READY_TIME;
        if let Some((w, _)) = self.current() {
            println!("weapon: {}", w.def.name);
        }
    }

    fn magnification(&self) -> f32 {
        self.current()
            .map(|(w, s)| w.def.magnification(s.zoom))
            .unwrap_or(1.0)
    }

    fn update(&mut self, dt: f32) {
        if self.walking {
            let held = |k: KeyCode| self.keys.contains(&k);
            let axis =
                |pos: KeyCode, neg: KeyCode| held(pos) as i32 as f32 - held(neg) as i32 as f32;
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
                self.player.velocity = Vec3::ZERO;
            }
            self.camera.position = self.player.eye();
            let speed = self.player.velocity.truncate().length();
            if self.player.grounded {
                self.bob_phase += speed * dt * 4.5;
            }
        } else {
            self.camera.update(&self.keys, dt);
        }

        // Weapon.
        self.readying = (self.readying - dt).max(0.0);
        let input = WeaponInput {
            fire: (self.fire_held || self.fire_tapped) && self.captured && self.readying <= 0.0,
            reload: self.keys.contains(&KeyCode::KeyR),
            zoom: self.zoom_held || self.zoom_tapped || self.keys.contains(&KeyCode::KeyZ),
        };
        self.fire_tapped = false;
        self.zoom_tapped = false;
        let (eye, (f, r, u)) = (self.camera.position, self.camera.basis());
        if let (Some(w), Some(state)) = (
            self.scene.weapons.get(self.weapon),
            self.weapon_states.get_mut(self.weapon),
        ) {
            for shot in state.update(&w.def, input, dt) {
                let dir = shot.direction(f, r, u);
                if let Some((t, n)) = self.world.raycast_hit(eye, dir, w.def.range) {
                    self.effects.impact(eye + dir * t, n);
                }
            }
        }
        self.effects.update(dt);
    }

    /// Where the first person weapon sits: camera space is x forward, y left, z up.
    fn view_model_matrix(&self, weapon: &WeaponAssets, state: &WeaponState) -> Mat4 {
        let def = &weapon.def;
        let (f, r, u) = self.camera.basis();
        let cam = Mat4::from_cols(
            f.extend(0.0),
            (-r).extend(0.0),
            u.extend(0.0),
            self.camera.position.extend(1.0),
        );
        let speed = (self.player.velocity.truncate().length() / 2.25).min(1.0);
        let bob = if self.walking && self.player.grounded {
            speed
        } else {
            0.0
        };
        // Until first person animations pose them, every gun holds its root
        // node at the same spot: low on the right, a little ahead of the eye.
        let mut offset = Vec3::new(0.105, -0.048, -0.068) - Vec3::from(weapon.grip);
        offset.y += self.bob_phase.sin() * 0.004 * bob;
        offset.z -= self.bob_phase.cos().abs() * 0.003 * bob;
        // Kick back after each shot.
        offset.x -= 0.012 * (-state.since_shot * 18.0).exp();
        // Dip while reloading and while bringing the weapon up.
        let mut dip = 0.0;
        if let Some(left) = state.reloading {
            let t = 1.0 - left / def.reload_time.max(0.01);
            dip = smoothstep(t * 4.0).min(smoothstep((1.0 - t) * 4.0));
        }
        dip = dip.max(self.readying / READY_TIME);
        offset.z -= dip * 0.05;
        cam * Mat4::from_translation(offset)
            * Mat4::from_rotation_z(0.04)
            * Mat4::from_rotation_y(dip * 0.6)
    }

    fn build_hud(&self, w: f32, h: f32) -> Vec<gpu::HudBatch> {
        let mut hb = HudBuilder::new(w, h);
        let scene = &self.scene;
        let mut drew_tracker = false;
        for widget in &scene.player_hud {
            match widget.name.as_str() {
                "motion_tracker_background" if !drew_tracker => {
                    drew_tracker = true;
                    hb.widget(widget, hud::BLUE, hud_mode::CHANNELS, 0.0);
                }
                "shield_meter" => hb.widget(widget, hud::BLUE, hud_mode::METER_GREY, 1.0),
                "shield_mask" => hb.widget(widget, hud::BLUE, hud_mode::PLAIN, 0.0),
                "frag_grenade_default" => {
                    hb.widget(widget, hud::BLUE, hud_mode::CHANNELS, 0.0);
                    // Plasma grenades in the left box, frags (selected) in the right.
                    let rect = hb.widget_rect(widget);
                    let th = (rect[3] - rect[1]) * 0.5;
                    for (at, count) in [(0.18, "0"), (0.69, "2")] {
                        hb.text(
                            scene.hud_font,
                            [rect[0] + (rect[2] - rect[0]) * at, rect[1] + th * 0.5],
                            th,
                            count,
                            hud::BLUE,
                        );
                    }
                }
                _ => {}
            }
        }
        let Some((weapon, state)) = self.current() else {
            return hb.finish();
        };
        let zoomed = state.zoom > 0;
        let def = &weapon.def;
        let magnification = def.magnification(state.zoom);
        let ammo_fill = if def.uses_ammo() {
            state.loaded as f32 / def.magazine_size.max(1) as f32
        } else {
            1.0
        };
        for widget in &weapon.hud {
            match hud_role(&widget.name, magnification) {
                HudRole::Scope if zoomed => {
                    hb.scope(widget, scene.hud_white, [0.0, 0.0, 0.0, 132.0 / 255.0]);
                }
                HudRole::Background => {
                    hb.widget(widget, hud::BLUE, hud_mode::CHANNELS, 0.0);
                    if def.uses_ammo() {
                        let rect = hb.widget_rect(widget);
                        let th = (rect[3] - rect[1]) * 0.5;
                        hb.text(
                            scene.hud_font,
                            [rect[0] + (rect[2] - rect[0]) * 0.19, rect[1] + th * 0.5],
                            th,
                            &state.reserve.to_string(),
                            hud::BLUE,
                        );
                    }
                }
                HudRole::AmmoMeter => hb.widget(widget, hud::BLUE, hud_mode::METER_BLUE, ammo_fill),
                HudRole::ZoomedAmmoMeter if zoomed => {
                    hb.widget(widget, hud::BLUE, hud_mode::METER_BLUE, ammo_fill)
                }
                HudRole::Static => hb.widget(widget, hud::BLUE, hud_mode::CHANNELS, 0.0),
                HudRole::Zoomed if zoomed => hb.widget(widget, hud::BLUE, hud_mode::CHANNELS, 0.0),
                _ => {}
            }
        }
        let s = hb.scale();
        if def.uses_ammo() && state.loaded == 0 && state.reloading.is_none() {
            let msg = if state.reserve == 0 {
                "NO AMMO"
            } else {
                "RELOAD"
            };
            hb.text(
                scene.hud_font,
                [w * 0.5, h * 0.5 + 48.0 * s],
                12.0 * s,
                msg,
                hud::RED,
            );
        }
        if self.readying > 0.0 {
            hb.text(
                scene.hud_font,
                [w * 0.5, h * 0.5 + 48.0 * s],
                10.0 * s,
                &def.name.replace('_', " "),
                hud::BLUE,
            );
        }
        hb.finish()
    }

    fn render(&mut self) {
        let Some(g) = &self.gpu else { return };
        let (w, h) = g.size();
        let aspect = g.aspect();
        let magnification = self.magnification();
        let view_proj = self.camera.view_proj(aspect, magnification);
        let (_, r, u) = self.camera.basis();
        let world = [DrawCall {
            mesh: 0,
            model: Mat4::IDENTITY,
        }];
        let sprites = self.effects.sprites(r, u);
        let mut view_models = Vec::new();
        let mut view_sprites = Vec::new();
        if let Some((weapon, state)) = self.current() {
            if let (Some(mesh), 0) = (weapon.view_mesh, state.zoom) {
                let model = self.view_model_matrix(weapon, state);
                view_models.push(DrawCall { mesh, model });
                if state.since_shot < 0.05 {
                    let muzzle = model.transform_point3(Vec3::from(weapon.muzzle));
                    let size = 0.012 + 0.006 * (state.since_shot * 300.0).sin().abs();
                    effects::quad(
                        &mut view_sprites,
                        muzzle,
                        r * size,
                        u * size,
                        [1.0, 0.8, 0.4, 0.9],
                    );
                }
            }
        }
        let view_model_proj = camera::projection(aspect, 1.0, 0.005, 10.0) * self.camera.view();
        let hud = self.build_hud(w, h);
        let frame = Frame {
            view_proj,
            camera: self.camera.position,
            world: &world,
            sprites: &sprites,
            view_model_proj,
            view_models: &view_models,
            view_sprites: &view_sprites,
            hud: &hud,
        };
        if let Some(g) = &mut self.gpu {
            g.render(&frame);
        }
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
            WindowEvent::MouseInput { state, button, .. } => {
                let down = state == ElementState::Pressed;
                match button {
                    MouseButton::Left if down && !self.captured => self.set_capture(true),
                    MouseButton::Left => {
                        self.fire_held = down;
                        self.fire_tapped |= down;
                    }
                    MouseButton::Right => {
                        self.zoom_held = down && self.captured;
                        self.zoom_tapped |= self.zoom_held;
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } if self.captured => {
                let up = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y > 0.0,
                    MouseScrollDelta::PixelDelta(p) => p.y > 0.0,
                };
                let n = self.scene.weapons.len().max(1);
                self.switch_weapon(if up {
                    self.weapon + n - 1
                } else {
                    self.weapon + 1
                });
            }
            WindowEvent::Focused(false) => {
                self.set_capture(false);
                self.keys.clear();
                self.fire_held = false;
                self.zoom_held = false;
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                match event.state {
                    ElementState::Pressed => {
                        if !event.repeat {
                            self.key_pressed(code, event_loop);
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
                self.update(dt);
                self.render();
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
                let scale = 1.0 / self.magnification();
                self.camera.look(delta.0 as f32, delta.1 as f32, scale);
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }
}

impl App {
    fn key_pressed(&mut self, code: KeyCode, event_loop: &ActiveEventLoop) {
        match code {
            KeyCode::Tab => {
                self.walking = !self.walking;
                if self.walking {
                    // Drop in where the fly camera is.
                    let feet =
                        self.camera.position - Vec3::Z * self.player.biped.standing_camera_height;
                    self.player.position = feet;
                    self.player.velocity = Vec3::ZERO;
                }
            }
            KeyCode::Escape => {
                if self.captured {
                    self.set_capture(false);
                } else {
                    event_loop.exit();
                }
            }
            KeyCode::KeyQ => self.switch_weapon(self.weapon + 1),
            KeyCode::Digit1
            | KeyCode::Digit2
            | KeyCode::Digit3
            | KeyCode::Digit4
            | KeyCode::Digit5
            | KeyCode::Digit6
            | KeyCode::Digit7
            | KeyCode::Digit8
            | KeyCode::Digit9 => {
                let i = code as usize - KeyCode::Digit1 as usize;
                if i < self.scene.weapons.len() {
                    self.switch_weapon(i);
                }
            }
            _ => {}
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
        "{} triangles, {} textures, {} weapons",
        scene.triangle_count(),
        scene.textures.len() - 1,
        scene.weapons.len()
    );
    let camera = match scene.spawn {
        Some(s) => {
            let eye = Vec3::from(s.position) + Vec3::Z * scene.biped.standing_camera_height;
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
        camera.position - Vec3::Z * scene.biped.standing_camera_height + Vec3::Z * 0.05;
    let player = Player::new(spawn_point, scene.movement, scene.biped);
    let weapon_states = scene
        .weapons
        .iter()
        .map(|w| WeaponState::new(&w.def))
        .collect();

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        scene,
        world,
        player,
        walking,
        spawn_point,
        title: format!(
            "Halo 2 Rust: {name} (click to play, WASD move, mouse fire/zoom, R reload, Q switch weapon, Esc release)"
        ),
        window: None,
        gpu: None,
        camera,
        keys: HashSet::new(),
        captured: false,
        last_frame: Instant::now(),
        fire_held: false,
        zoom_held: false,
        fire_tapped: false,
        zoom_tapped: false,
        weapon: 0,
        weapon_states,
        readying: 0.0,
        effects: Effects::new(),
        bob_phase: 0.0,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hud_roles_follow_widget_names() {
        assert_eq!(
            hud_role("weapon_background_right", 1.0),
            HudRole::Background
        );
        assert_eq!(hud_role("weapon_background_left", 1.0), HudRole::Hidden);
        assert_eq!(hud_role("ammo_meter_single", 1.0), HudRole::AmmoMeter);
        assert_eq!(hud_role("scope_mask", 2.0), HudRole::Scope);
        assert_eq!(hud_role("5x", 5.0), HudRole::Zoomed);
        assert_eq!(hud_role("10x", 5.0), HudRole::Hidden);
        assert_eq!(hud_role("crosshair_friendly", 1.0), HudRole::Hidden);
    }
}
