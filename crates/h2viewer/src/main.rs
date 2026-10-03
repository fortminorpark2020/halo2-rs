//! h2viewer: play a Halo 2 multiplayer map, alone against bots or with
//! friends in splitscreen.
//!
//! Usage: h2viewer [path\to\level.map]
//! With no argument it looks for lockout.map in the usual install folders.
//!
//! Keyboard and mouse: click to capture the mouse, WASD move, Space jump,
//! Ctrl/C crouch, left mouse fire, right mouse / Z zoom, R reload, F melee,
//! G / middle mouse throw a grenade, X switch grenades, E pick up (hold to
//! swap weapons), Q / mouse wheel switch weapon, B add a bot, 1-9 take any
//! weapon (testing), Tab toggles walking / flying (fly: Space/C up/down, Shift
//! fast), Esc releases the mouse (Esc again quits).
//!
//! Controllers (Halo 2's layout, see `input`): A takes over player one,
//! Start joins as another splitscreen player, Back leaves.

mod body;
mod camera;
mod effects;
mod font;
mod gpu;
mod hud;
mod input;
mod local;
mod probe;
mod rig;
mod scene;

use blam_cache::geometry::Mesh;
use blam_cache::PlayerSpawn;
use body::{BodyAnimator, BodyInput};
use camera::FlyCamera;
use effects::Effects;
use gilrs::GamepadId;
use glam::{Mat4, Vec3};
use gpu::{DrawCall, Frame};
use h2sim::game::{Event, GrenadeKind, HeldWeapon, TICK};
use h2sim::{Bot, Command, Game, ItemKind, ItemSpawn, NavGraph, Rules, WeaponState, World};
use input::{PadPress, Pads};
use local::{armor_colors, display_name, kill_message, Keyboard, LocalPlayer, Taps};
use scene::Scene;
use std::collections::HashSet;
use std::f32::consts::FRAC_PI_2;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

/// Most people sharing one screen.
const MAX_LOCAL: usize = 4;

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

/// Slayer on the map's weapons: spawn with a Battle Rifle and an SMG.
fn rules(scene: &Scene) -> Rules {
    let index = |names: &[&str]| -> Vec<usize> {
        names
            .iter()
            .filter_map(|n| scene.weapons.iter().position(|w| w.def.name == *n))
            .collect()
    };
    let defaults = Rules::default();
    let mut frag = defaults.frag;
    let mut plasma = defaults.plasma;
    if let Some(v) = scene.grenades[0].speed {
        frag.speed = v;
    }
    if let Some(v) = scene.grenades[1].speed {
        plasma.speed = v;
    }
    Rules {
        starting_weapons: index(&["battle_rifle", "smg"]),
        headshot_weapons: index(&[
            "battle_rifle",
            "magnum",
            "covenant_carbine",
            "sniper_rifle",
            "beam_rifle",
        ]),
        lunge_weapons: index(&["energy_blade"]),
        frag,
        plasma,
        ..defaults
    }
}

/// A Spartan's body posed for this frame.
struct BodyPose {
    vertices: Vec<scene::Vertex>,
    object: Mat4,
    weapon: Mat4,
}

struct App {
    scene: Scene,
    world: World,
    game: Game,
    /// Computer players and the player each one drives.
    bots: Vec<(usize, Bot)>,
    nav: NavGraph,
    /// The people playing at this computer, one view each.
    locals: Vec<LocalPlayer>,
    pads: Pads,
    title: String,
    window: Option<Arc<Window>>,
    gpu: Option<gpu::Gpu>,
    keys: HashSet<KeyCode>,
    captured: bool,
    fire_held: bool,
    zoom_held: bool,
    last_frame: Instant,
    /// Time not yet simulated, less than a tick.
    pending: f32,
    effects: Effects,
    /// Third person animation of each player.
    bodies: Vec<BodyAnimator>,
    /// Actions players started this frame (reload, melee...), for their bodies.
    body_actions: Vec<(usize, &'static str)>,
    body_poses: Vec<Option<BodyPose>>,
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

    /// The player at the keyboard and mouse.
    fn keyboard_local(&mut self) -> Option<&mut LocalPlayer> {
        self.locals.iter_mut().find(|l| l.keyboard)
    }

    fn local_of(&mut self, player: usize) -> Option<&mut LocalPlayer> {
        self.locals.iter_mut().find(|l| l.player == player)
    }

    /// Tell everyone at this computer.
    fn announce(&mut self, text: &str) {
        println!("{}", text.to_lowercase());
        for l in &mut self.locals {
            l.message(text.to_string());
        }
    }

    /// For testing: swap the weapon in hand for any weapon.
    fn give_weapon(&mut self, player: usize, w: usize) {
        let Some(def) = self.scene.weapons.get(w).map(|a| &a.def) else {
            return;
        };
        let p = &mut self.game.players[player];
        if !p.alive || p.weapons.iter().any(|h| h.weapon == w) {
            return;
        }
        let held = HeldWeapon {
            weapon: w,
            state: WeaponState::new(def),
        };
        match p.weapons.get_mut(p.current) {
            Some(h) => *h = held,
            None => p.weapons.push(held),
        }
        println!("weapon: {}", def.name);
    }

    fn add_bot(&mut self) {
        if self.game.players.len() >= scene::MAX_BODIES {
            return;
        }
        let i = self.game.add_player();
        self.bots.push((i, Bot::new(i as u32 * 7919 + 13)));
        self.announce(&format!("PLAYER {} JOINED", i + 1));
    }

    /// Another person joins in splitscreen.
    fn add_local(&mut self, pad: Option<GamepadId>) {
        if self.locals.len() >= MAX_LOCAL || self.game.players.len() >= scene::MAX_BODIES {
            return;
        }
        let i = self.game.add_player();
        let mut l = LocalPlayer::new(i, &self.game);
        l.pad = pad;
        self.locals.push(l);
        self.announce(&format!("PLAYER {} JOINED", i + 1));
    }

    fn pad_pressed(&mut self, id: GamepadId, press: PadPress) {
        let owner = self.locals.iter().position(|l| l.pad == Some(id));
        match (owner, press) {
            (None, PadPress::Claim) => {
                if let Some(l) = self
                    .locals
                    .iter_mut()
                    .find(|l| l.keyboard && l.pad.is_none())
                {
                    l.pad = Some(id);
                    l.message("CONTROLLER CONNECTED".into());
                }
            }
            (None, PadPress::Join) => self.add_local(Some(id)),
            (Some(k), PadPress::Leave) => {
                if self.locals[k].keyboard {
                    self.locals[k].pad = None;
                } else {
                    // Their Spartan plays on as a bot.
                    let l = self.locals.remove(k);
                    self.bots
                        .push((l.player, Bot::new(l.player as u32 * 7919 + 13)));
                    self.announce(&format!("PLAYER {} LEFT", l.player + 1));
                }
            }
            (Some(k), p) => {
                let t = &mut self.locals[k].taps;
                match p {
                    PadPress::Fire => t.fire = true,
                    PadPress::Melee => t.melee = true,
                    PadPress::Reload => t.reload = true,
                    PadPress::SwitchWeapon => t.switch_weapon = true,
                    PadPress::Grenade => t.throw_grenade = true,
                    PadPress::SwitchGrenade => t.switch_grenade = true,
                    PadPress::Zoom => t.zoom = true,
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn update(&mut self, dt: f32) {
        for (id, press) in self.pads.presses() {
            self.pad_pressed(id, press);
        }
        for l in &mut self.locals {
            if let Some(state) = l.pad.and_then(|id| self.pads.state(id)) {
                let scale = 1.0 / l.magnification(&self.scene, &self.game);
                l.camera.look_stick(state.right, dt, scale);
            }
            if l.flying {
                l.camera.update(&self.keys, dt);
            }
        }
        // The game advances in fixed ticks.
        self.pending += dt;
        let mut ticked = false;
        while self.pending >= TICK {
            self.pending -= TICK;
            let mut commands = vec![Command::default(); self.game.players.len()];
            let keyboard = Keyboard {
                keys: &self.keys,
                captured: self.captured,
                fire_held: self.fire_held,
                zoom_held: self.zoom_held,
            };
            for l in &self.locals {
                let pad = l.pad.and_then(|id| self.pads.state(id));
                commands[l.player] = l.command(l.keyboard.then_some(&keyboard), pad);
            }
            for (i, bot) in &mut self.bots {
                commands[*i] = bot.think(&self.game, &self.world, &self.nav, *i);
            }
            self.game.step(&self.world, &commands);
            if !ticked {
                for l in &mut self.locals {
                    l.taps = Taps::default();
                }
                ticked = true;
            }
            for l in &mut self.locals {
                let eye = self.game.players[l.player].eye();
                l.eyes = (l.eyes.1, eye);
            }
            self.handle_events();
        }
        for l in &mut self.locals {
            l.update_camera(&self.game, &self.world, self.pending, dt);
        }
        self.animate_bodies(dt);
        for l in &mut self.locals {
            l.animate_view_model(&self.scene, &mut self.game, dt);
        }
        self.effects.update(dt);
    }

    fn handle_events(&mut self) {
        for e in std::mem::take(&mut self.game.events) {
            match e {
                Event::Shot {
                    player,
                    hit,
                    hit_player,
                    ..
                } => {
                    if let Some(l) = self.local_of(player) {
                        l.view.fired = true;
                    }
                    match (hit, hit_player) {
                        (Some((p, _)), Some(j)) => {
                            let shielded = self.game.players[j].shield > 0.0;
                            self.effects.player_hit(p, shielded);
                        }
                        (Some((p, n)), None) => self.effects.impact(p, n),
                        _ => {}
                    }
                }
                Event::Reloaded { player, empty } => {
                    self.body_actions.push((player, "reload_1"));
                    if let Some(l) = self.local_of(player) {
                        l.view.reload = Some(empty);
                    }
                }
                Event::Switched { player } => {
                    self.body_actions.push((player, "ready"));
                    if let Some(l) = self.local_of(player) {
                        l.view.switched = true;
                    }
                }
                Event::Melee { player, .. } => {
                    self.body_actions.push((player, "melee_strike_1"));
                    if let Some(l) = self.local_of(player) {
                        l.view.melee = true;
                    }
                }
                Event::Thrown { player } => {
                    self.body_actions.push((player, "throw_grenade"));
                    if let Some(l) = self.local_of(player) {
                        l.view.thrown = true;
                    }
                }
                Event::Exploded { kind, position } => {
                    self.effects
                        .explosion(position, kind == GrenadeKind::Plasma);
                }
                Event::Killed { killer, victim, .. } => {
                    println!(
                        "{}",
                        kill_message(usize::MAX, killer, victim).to_lowercase()
                    );
                    for l in &mut self.locals {
                        l.message(kill_message(l.player, killer, victim));
                        if victim == l.player {
                            // The death camera starts behind and above the body.
                            l.camera.pitch = -0.6;
                        }
                    }
                }
                Event::Spawned { player } => {
                    let p = &self.game.players[player];
                    let (eye, yaw) = (p.eye(), p.yaw);
                    if let Some(l) = self.local_of(player) {
                        l.eyes = (eye, eye);
                        l.camera.yaw = yaw;
                        l.camera.pitch = 0.0;
                        l.view.switched = true;
                    }
                }
                Event::PickedUp { player, kind } => {
                    let what = match kind {
                        ItemKind::Weapon(w) => self
                            .scene
                            .weapons
                            .get(w)
                            .map(|a| display_name(&a.def.name))
                            .unwrap_or_default(),
                        ItemKind::FragGrenades => "FRAG GRENADE".into(),
                        ItemKind::PlasmaGrenades => "PLASMA GRENADE".into(),
                    };
                    if let Some(l) = self.local_of(player) {
                        l.message(format!("PICKED UP {what}"));
                    }
                }
                _ => {}
            }
        }
    }

    /// Pose every Spartan's body. Each view leaves out its own player's
    /// body while it looks through their eyes.
    fn animate_bodies(&mut self, dt: f32) {
        let actions = std::mem::take(&mut self.body_actions);
        let Some(body) = &self.scene.body else {
            return;
        };
        let rig = &body.rig;
        let n = self.game.players.len().min(scene::MAX_BODIES);
        self.bodies.resize_with(n, BodyAnimator::default);
        self.body_poses.resize_with(n, || None);
        for (i, p) in self.game.players.iter().enumerate().take(n) {
            let style = p
                .held()
                .and_then(|h| self.scene.weapons.get(h.weapon))
                .map_or(("rifle", "any"), |w| body::weapon_style(&w.def.name));
            let (s, c) = p.yaw.sin_cos();
            let v = p.body.velocity.truncate();
            let input = BodyInput {
                velocity: glam::vec2(v.x * c + v.y * s, v.y * c - v.x * s),
                grounded: p.body.grounded,
                crouching: p.body.crouch > 0.5,
                alive: p.alive,
                style,
            };
            for &(_, what) in actions.iter().filter(|a| a.0 == i) {
                self.bodies[i].act(rig, &input, what);
            }
            let pose = self.bodies[i].update(rig, &input, dt);
            let world = rig.world(&pose);
            self.body_poses[i] = Some(BodyPose {
                vertices: rig.skin.pose(&rig.skin_matrices(&world)),
                object: Mat4::from_translation(p.body.position) * Mat4::from_rotation_z(p.yaw),
                weapon: rig.weapon_frame(&world),
            });
        }
    }

    /// Everything in the world besides the level: scenery, items lying on
    /// the map, dropped weapons and grenades in flight.
    fn world_draws(&self) -> Vec<DrawCall> {
        let scene = &self.scene;
        let light_at = |p: Vec3| scene.level_light.at(&scene.textures, p + Vec3::Z * 0.2);
        let mut world = vec![DrawCall {
            mesh: 0,
            model: Mat4::IDENTITY,
            light: None,
            colors: None,
        }];
        for o in &scene.objects {
            world.push(DrawCall {
                mesh: o.mesh,
                model: o.transform,
                light: o.light,
                colors: None,
            });
        }
        for (item, timer) in scene.items.iter().zip(&self.game.item_timers) {
            if let (Some(mesh), true) = (item.mesh, *timer <= 0.0) {
                world.push(DrawCall {
                    mesh,
                    model: item.transform,
                    light: item.light,
                    colors: None,
                });
            }
        }
        for d in &self.game.dropped {
            if let Some(mesh) = scene.weapons.get(d.weapon).and_then(|w| w.world_mesh) {
                world.push(DrawCall {
                    mesh,
                    model: scene::placement_matrix(d.position.into(), [d.yaw, 0.0, FRAC_PI_2], 1.0),
                    light: light_at(d.position),
                    colors: None,
                });
            }
        }
        for g in &self.game.grenades {
            let assets = scene.grenades[(g.kind == GrenadeKind::Plasma) as usize];
            if let Some(mesh) = assets.mesh {
                // Tumbling through the air.
                let spin = if g.velocity.length_squared() > 0.01 {
                    self.game.time as f32 * 12.0
                } else {
                    0.0
                };
                world.push(DrawCall {
                    mesh,
                    model: Mat4::from_translation(g.position) * Mat4::from_rotation_y(spin),
                    light: light_at(g.position),
                    colors: None,
                });
            }
        }
        world
    }

    /// A player's body and the weapon in their hands.
    fn body_draws(&self, player: usize) -> Vec<DrawCall> {
        let (Some(body), Some(Some(pose))) = (&self.scene.body, self.body_poses.get(player)) else {
            return Vec::new();
        };
        let p = &self.game.players[player];
        let light = self
            .scene
            .level_light
            .at(&self.scene.textures, p.body.position + Vec3::Z * 0.2);
        let mut out = vec![DrawCall {
            mesh: body.meshes[player],
            model: pose.object,
            light,
            colors: Some(armor_colors(player)),
        }];
        let weapon = p.held().filter(|_| p.alive);
        if let Some(mesh) = weapon
            .and_then(|h| self.scene.weapons.get(h.weapon))
            .and_then(|w| w.world_mesh)
        {
            out.push(DrawCall {
                mesh,
                model: pose.object * pose.weapon,
                light,
                colors: None,
            });
        }
        out
    }

    fn render(&mut self) {
        let Some(g) = &self.gpu else { return };
        let (w, h) = g.size();
        let ports = local::viewports(self.locals.len(), w as u32, h as u32);
        let shared = self.world_draws();
        // Posed bodies go up once, with the first view.
        let mut body_meshes = Vec::new();
        if let Some(body) = &self.scene.body {
            for (i, pose) in self.body_poses.iter().enumerate() {
                if let Some(pose) = pose {
                    body_meshes.push((body.meshes[i], pose.vertices.clone()));
                }
            }
        }
        struct View {
            viewport: [u32; 4],
            aspect: f32,
            magnification: f32,
            camera: FlyCamera,
            world: Vec<DrawCall>,
            sprites: Vec<gpu::SpriteVertex>,
            draws: local::ViewDraws,
            hud: Vec<gpu::HudBatch>,
            view_model_proj: Mat4,
        }
        let mut views = Vec::new();
        for (k, l) in self.locals.iter().enumerate() {
            let Some(&viewport) = ports.get(k) else {
                break;
            };
            let aspect = viewport[2] as f32 / viewport[3].max(1) as f32;
            let mut world = shared.clone();
            let own = l.first_person(&self.game).then_some(l.player);
            for i in 0..self.body_poses.len() {
                if Some(i) != own {
                    world.extend(self.body_draws(i));
                }
            }
            let (_, r, u) = l.camera.basis();
            let mut draws = l.view_draws(&self.scene, &self.game);
            if k == 0 {
                draws.posed.splice(0..0, std::mem::take(&mut body_meshes));
            }
            views.push(View {
                viewport,
                aspect,
                magnification: l.magnification(&self.scene, &self.game),
                camera: FlyCamera {
                    position: l.camera.position,
                    yaw: l.camera.yaw,
                    pitch: l.camera.pitch,
                },
                world,
                sprites: self.effects.sprites(r, u),
                draws,
                hud: l.build_hud(
                    &self.scene,
                    &self.game,
                    viewport[2] as f32,
                    viewport[3] as f32,
                ),
                view_model_proj: l.view_model_proj(aspect),
            });
        }
        let frames: Vec<Frame> = views
            .iter()
            .map(|v| {
                let sky_view =
                    glam::camera::rh::view::look_to_mat4(Vec3::ZERO, v.camera.forward(), Vec3::Z);
                Frame {
                    viewport: v.viewport,
                    posed: &v.draws.posed,
                    sky: self.scene.sky.map(|mesh| DrawCall {
                        mesh,
                        model: Mat4::IDENTITY,
                        light: None,
                        colors: None,
                    }),
                    sky_proj: camera::projection(v.aspect, v.magnification, 1.0, 10000.0)
                        * sky_view,
                    view_proj: v.camera.view_proj(v.aspect, v.magnification),
                    camera: v.camera.position,
                    world: &v.world,
                    sprites: &v.sprites,
                    view_model_proj: v.view_model_proj,
                    view_models: &v.draws.view_models,
                    view_sprites: &v.draws.view_sprites,
                    hud: &v.hud,
                }
            })
            .collect();
        if let Some(g) = &mut self.gpu {
            g.render(&frames);
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
                let captured = self.captured;
                match button {
                    MouseButton::Left if down && !captured => self.set_capture(true),
                    MouseButton::Left => {
                        self.fire_held = down;
                        if let Some(l) = self.keyboard_local() {
                            l.taps.fire |= down;
                        }
                    }
                    MouseButton::Right => {
                        self.zoom_held = down && captured;
                        if let Some(l) = self.keyboard_local() {
                            l.taps.zoom |= down && captured;
                        }
                    }
                    MouseButton::Middle => {
                        if let Some(l) = self.keyboard_local() {
                            l.taps.throw_grenade |= down && captured;
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { .. } if self.captured => {
                if let Some(l) = self.keyboard_local() {
                    l.taps.switch_weapon = true;
                }
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
            if !self.captured {
                return;
            }
            let Some(k) = self.locals.iter().position(|l| l.keyboard) else {
                return;
            };
            let scale = 1.0 / self.locals[k].magnification(&self.scene, &self.game);
            self.locals[k]
                .camera
                .look(delta.0 as f32, delta.1 as f32, scale);
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
        if code == KeyCode::Escape {
            if self.captured {
                self.set_capture(false);
            } else {
                event_loop.exit();
            }
            return;
        }
        if code == KeyCode::KeyB {
            self.add_bot();
            return;
        }
        let Some(k) = self.locals.iter().position(|l| l.keyboard) else {
            return;
        };
        let player = self.locals[k].player;
        let l = &mut self.locals[k];
        match code {
            KeyCode::Tab => {
                l.flying = !l.flying;
                if !l.flying {
                    // Drop in where the fly camera is.
                    let p = &mut self.game.players[player];
                    p.body.position =
                        l.camera.position - Vec3::Z * p.body.biped.standing_camera_height;
                    p.body.velocity = Vec3::ZERO;
                    l.eyes = (p.eye(), p.eye());
                }
            }
            KeyCode::KeyQ => l.taps.switch_weapon = true,
            KeyCode::KeyF => l.taps.melee = true,
            KeyCode::KeyR => l.taps.reload = true,
            KeyCode::KeyG => l.taps.throw_grenade = true,
            KeyCode::KeyX => l.taps.switch_grenade = true,
            KeyCode::Digit1
            | KeyCode::Digit2
            | KeyCode::Digit3
            | KeyCode::Digit4
            | KeyCode::Digit5
            | KeyCode::Digit6
            | KeyCode::Digit7
            | KeyCode::Digit8
            | KeyCode::Digit9 => {
                self.give_weapon(player, code as usize - KeyCode::Digit1 as usize);
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
        "{} triangles, {} textures, {} weapons, {} items",
        scene.triangle_count(),
        scene.textures.len() - 1,
        scene.weapons.len(),
        scene.items.len()
    );
    // H2_POS="x y z yaw_degrees" starts somewhere else (for testing).
    let start = std::env::var("H2_POS").ok().and_then(|v| {
        let n: Vec<f32> = v
            .split_whitespace()
            .filter_map(|x| x.parse().ok())
            .collect();
        (n.len() == 4).then(|| PlayerSpawn {
            position: [n[0], n[1], n[2]],
            facing: n[3].to_radians(),
        })
    });
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    let world = World::new(&scene.collision.positions, &scene.collision.indices);
    let walking = start.is_some() || !scene.spawns.is_empty();
    let (focus, radius) = level_focus(&scene.collision);
    let mut spawns: Vec<(Vec3, f32)> = scene
        .spawns
        .iter()
        .map(|s| (Vec3::from(s.position), s.facing))
        .collect();
    if spawns.is_empty() {
        spawns.push((focus, 0.0));
    }
    let items = scene
        .items
        .iter()
        .map(|i| ItemSpawn {
            kind: i.kind,
            position: i.position,
            respawn: i.respawn_seconds,
        })
        .collect();
    let mut game = Game::new(
        rules(&scene),
        scene.weapons.iter().map(|w| w.def.clone()).collect(),
        spawns,
        items,
        scene.movement,
        scene.biped,
    );
    let me = game.add_player();
    if let Some(s) = start {
        let p = &mut game.players[me];
        p.body.position = Vec3::from(s.position) + Vec3::Z * 0.05;
        p.yaw = s.facing;
    }
    game.events.clear();
    let mut player_one = LocalPlayer::new(me, &game);
    player_one.keyboard = true;
    if !walking {
        // No spawn points: look over the level from above.
        player_one.flying = true;
        player_one.camera = FlyCamera::looking_at(
            focus + glam::vec3(radius * 0.6, -radius * 0.6, radius * 0.4),
            focus,
        );
    }
    let mut nav_points: Vec<Vec3> = game.spawns.iter().map(|s| s.0).collect();
    nav_points.extend(game.item_spawns.iter().map(|i| i.position));
    let nav = NavGraph::build(&world, &nav_points);
    println!(
        "bot routes: {} points, {} links",
        nav.points.len(),
        nav.links.iter().map(Vec::len).sum::<usize>()
    );

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        scene,
        world,
        game,
        bots: Vec::new(),
        nav,
        locals: vec![player_one],
        pads: Pads::new(),
        title: format!(
            "Halo 2 Rust: {name} (click to play, WASD move, mouse fire/zoom, G grenade, E pick up, F melee, R reload, Q switch weapon, B add bot, controller Start joins, Esc release)"
        ),
        window: None,
        gpu: None,
        keys: HashSet::new(),
        captured: false,
        fire_held: false,
        zoom_held: false,
        last_frame: Instant::now(),
        pending: 0.0,
        effects: Effects::new(),
        bodies: Vec::new(),
        body_actions: Vec::new(),
        body_poses: Vec::new(),
    };
    // H2_SPLIT=<n> starts with n people in splitscreen (for testing).
    let split = std::env::var("H2_SPLIT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(1);
    for _ in 1..split {
        app.add_local(None);
    }
    // Three computer opponents (H2_BOTS=<n> for another number).
    let bots = std::env::var("H2_BOTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    for _ in 0..bots {
        app.add_bot();
    }
    // H2_WEAPON=<n> starts with another weapon in hand (for testing).
    if let Some(n) = std::env::var("H2_WEAPON").ok().and_then(|v| v.parse().ok()) {
        app.give_weapon(me, n);
    }
    event_loop.run_app(&mut app)?;
    Ok(())
}
