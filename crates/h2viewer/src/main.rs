//! h2viewer: play a Halo 2 multiplayer map.
//!
//! Usage: h2viewer [path\to\level.map]
//! With no argument it looks for lockout.map in the usual install folders.
//!
//! Controls: click to capture the mouse, WASD move, Space jump, Ctrl/C crouch,
//! left mouse fire, right mouse / Z zoom, R reload, F melee, G / middle mouse
//! throw a grenade, X switch grenades, E pick up (hold to swap weapons), Q /
//! mouse wheel switch weapon, 1-9 take any weapon (testing), Tab toggles
//! walking / flying (fly: Space/C up/down, Shift fast), Esc releases the
//! mouse (Esc again quits).

mod body;
mod camera;
mod effects;
mod font;
mod gpu;
mod hud;
mod probe;
mod rig;
mod scene;

use blam_cache::geometry::Mesh;
use blam_cache::PlayerSpawn;
use body::{BodyAnimator, BodyInput};
use camera::FlyCamera;
use effects::Effects;
use glam::{Mat4, Vec3};
use gpu::{hud_mode, DrawCall, Frame};
use h2sim::game::{Event, GrenadeKind, HeldWeapon, Spartan, TICK};
use h2sim::{Bot, Command, Game, ItemKind, ItemSpawn, NavGraph, Rules, WeaponState, World};
use hud::HudBuilder;
use scene::{Scene, WeaponAssets};
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

/// Alternate between a weapon's melee swings.
fn melee_strike(rig: &rig::FirstPersonRig, n: usize) -> Option<usize> {
    rig.find(&format!("first_person:melee_strike_{}", 1 + n % 2), 0)
        .or_else(|| rig.find("first_person:melee_strike_1", 0))
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Free-for-all armour colours, in player order: Halo 2's red, blue, green,
/// orange, purple, gold, brown, pink, white and black.
const ARMOR_COLORS: [[f32; 3]; 10] = [
    [0.62, 0.10, 0.10],
    [0.15, 0.27, 0.65],
    [0.22, 0.45, 0.16],
    [0.88, 0.45, 0.12],
    [0.40, 0.20, 0.58],
    [0.82, 0.64, 0.16],
    [0.40, 0.27, 0.15],
    [0.92, 0.52, 0.64],
    [0.85, 0.85, 0.85],
    [0.13, 0.13, 0.14],
];

/// A player's primary and secondary armour colours.
fn armor_colors(player: usize) -> [[f32; 3]; 2] {
    let primary = ARMOR_COLORS[player % ARMOR_COLORS.len()];
    let secondary = ARMOR_COLORS[(player + 8) % ARMOR_COLORS.len()];
    [primary, secondary]
}

/// "battle_rifle" -> "BATTLE RIFLE".
fn display_name(name: &str) -> String {
    name.replace('_', " ").to_uppercase()
}

fn player_name(me: usize, i: usize) -> String {
    if i == me {
        "YOU".into()
    } else {
        format!("PLAYER {}", i + 1)
    }
}

/// A kill feed line, as Halo 2 words it.
fn kill_message(me: usize, killer: Option<usize>, victim: usize) -> String {
    let v = player_name(me, victim);
    match killer {
        Some(k) if k == victim && k == me => "YOU KILLED YOURSELF".into(),
        Some(k) if k == victim => format!("{v} COMMITTED SUICIDE"),
        Some(k) => format!("{} KILLED {v}", player_name(me, k)),
        None => format!("{v} DIED"),
    }
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

/// Presses since the last game tick, so a quick click between ticks still counts.
#[derive(Default)]
struct Taps {
    fire: bool,
    zoom: bool,
    melee: bool,
    reload: bool,
    switch_weapon: bool,
    throw_grenade: bool,
    switch_grenade: bool,
}

/// What the local player's first person view should react to this frame.
#[derive(Default)]
struct ViewEvents {
    fired: bool,
    melee: bool,
    thrown: bool,
    reload: Option<bool>,
    switched: bool,
}

struct App {
    scene: Scene,
    world: World,
    game: Game,
    /// The player at this keyboard and mouse.
    me: usize,
    /// Computer players and the player each one drives.
    bots: Vec<(usize, Bot)>,
    nav: NavGraph,
    walking: bool,
    title: String,
    window: Option<Arc<Window>>,
    gpu: Option<gpu::Gpu>,
    camera: FlyCamera,
    keys: HashSet<KeyCode>,
    captured: bool,
    last_frame: Instant,
    fire_held: bool,
    zoom_held: bool,
    taps: Taps,
    /// Time not yet simulated, less than a tick.
    pending: f32,
    /// Eye position after the previous and the latest tick, for smooth
    /// motion between ticks.
    eyes: (Vec3, Vec3),
    effects: Effects,
    bob_phase: f32,
    /// First person arms and gun animation.
    animator: rig::Animator,
    /// The weapon the first person animation belongs to.
    shown_weapon: Option<usize>,
    shots_fired: usize,
    /// Kill feed and pickups, newest last, with seconds left on screen.
    messages: Vec<(String, f32)>,
    /// Third person animation of each player.
    bodies: Vec<BodyAnimator>,
    /// Actions players started this frame (reload, melee...), for their bodies.
    body_actions: Vec<(usize, &'static str)>,
    /// Each player's posed body to draw: skinning matrices and the object
    /// matrix, and where their weapon goes.
    body_poses: Vec<Option<BodyPose>>,
}

struct BodyPose {
    skin: Vec<Mat4>,
    object: Mat4,
    weapon: Mat4,
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

    fn me(&self) -> &Spartan {
        &self.game.players[self.me]
    }

    fn current(&self) -> Option<(&WeaponAssets, &WeaponState)> {
        let held = self.me().held()?;
        Some((self.scene.weapons.get(held.weapon)?, &held.state))
    }

    fn magnification(&self) -> f32 {
        self.current()
            .map(|(w, s)| w.def.magnification(s.zoom))
            .unwrap_or(1.0)
    }

    /// For testing: swap the weapon in hand for any weapon.
    fn give_weapon(&mut self, w: usize) {
        let Some(def) = self.scene.weapons.get(w).map(|a| &a.def) else {
            return;
        };
        let p = &mut self.game.players[self.me];
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
        self.message(format!("{} JOINED", player_name(self.me, i)));
    }

    /// This tick's controls from the keyboard and mouse.
    fn command(&self) -> Command {
        let held = |k: KeyCode| self.keys.contains(&k);
        let axis = |pos: KeyCode, neg: KeyCode| held(pos) as i32 as f32 - held(neg) as i32 as f32;
        let mut cmd = Command {
            yaw: self.camera.yaw,
            pitch: self.camera.pitch,
            ..Command::default()
        };
        if !self.walking {
            return cmd;
        }
        let t = &self.taps;
        cmd.movement = glam::vec2(
            axis(KeyCode::KeyD, KeyCode::KeyA),
            axis(KeyCode::KeyW, KeyCode::KeyS),
        );
        cmd.jump = held(KeyCode::Space);
        cmd.crouch = held(KeyCode::ControlLeft) || held(KeyCode::KeyC);
        cmd.fire = (self.fire_held || t.fire) && self.captured;
        cmd.zoom = self.zoom_held || t.zoom || held(KeyCode::KeyZ);
        cmd.reload = held(KeyCode::KeyR) || t.reload;
        cmd.melee = t.melee;
        cmd.action = held(KeyCode::KeyE);
        cmd.switch_weapon = t.switch_weapon;
        cmd.throw_grenade = t.throw_grenade;
        cmd.switch_grenade = t.switch_grenade;
        cmd
    }

    fn update(&mut self, dt: f32) {
        if !self.walking {
            self.camera.update(&self.keys, dt);
        }
        // The game advances in fixed ticks.
        let mut view = ViewEvents::default();
        self.pending += dt;
        let mut ticked = false;
        while self.pending >= TICK {
            self.pending -= TICK;
            let mut commands = vec![Command::default(); self.game.players.len()];
            commands[self.me] = self.command();
            for (i, bot) in &mut self.bots {
                commands[*i] = bot.think(&self.game, &self.world, &self.nav, *i);
            }
            self.game.step(&self.world, &commands);
            if !ticked {
                self.taps = Taps::default();
                ticked = true;
            }
            self.eyes = (self.eyes.1, self.me().eye());
            self.handle_events(&mut view);
        }

        let me = &self.game.players[self.me];
        if self.walking {
            if me.alive {
                let a = (self.pending / TICK).clamp(0.0, 1.0);
                self.camera.position = self.eyes.0.lerp(self.eyes.1, a);
                if me.body.grounded {
                    self.bob_phase += me.body.velocity.truncate().length() * dt * 4.5;
                }
            } else {
                // Watch your body from behind until you respawn.
                let centre = me.body.position + Vec3::Z * 0.4;
                let back = -self.camera.forward();
                let dist = self
                    .world
                    .raycast(centre, back, 2.5)
                    .map_or(2.5, |t| (t - 0.2).max(0.2));
                self.camera.position = centre + back * dist;
            }
        }
        for m in &mut self.messages {
            m.1 -= dt;
        }
        self.messages.retain(|m| m.1 > 0.0);
        self.animate_bodies(dt);
        self.animate_view_model(dt, view);
        self.effects.update(dt);
    }

    fn handle_events(&mut self, view: &mut ViewEvents) {
        let me = self.me;
        for e in std::mem::take(&mut self.game.events) {
            match e {
                Event::Shot {
                    player,
                    hit,
                    hit_player,
                    ..
                } => {
                    view.fired |= player == me;
                    self.body_actions.push((player, "fire_1"));
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
                    if player == me {
                        view.reload = Some(empty);
                    }
                }
                Event::Switched { player } => {
                    self.body_actions.push((player, "ready"));
                    view.switched |= player == me;
                }
                Event::Melee { player, .. } => {
                    self.body_actions.push((player, "melee_strike_1"));
                    view.melee |= player == me;
                }
                Event::Thrown { player } => {
                    self.body_actions.push((player, "throw_grenade"));
                    view.thrown |= player == me;
                }
                Event::Exploded { kind, position } => {
                    self.effects
                        .explosion(position, kind == GrenadeKind::Plasma);
                }
                Event::Killed { killer, victim, .. } => {
                    self.message(kill_message(me, killer, victim));
                    if victim == me {
                        // The death camera starts behind and above the body.
                        self.camera.pitch = -0.6;
                    }
                }
                Event::Spawned { player } if player == me => {
                    let p = &self.game.players[me];
                    self.eyes = (p.eye(), p.eye());
                    self.camera.yaw = p.yaw;
                    self.camera.pitch = 0.0;
                    view.switched = true;
                }
                Event::PickedUp { player, kind } if player == me => {
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
                    self.message(format!("PICKED UP {what}"));
                }
                _ => {}
            }
        }
    }

    /// Pose every Spartan seen in third person: everyone but you, unless
    /// you are flying around or dead.
    fn animate_bodies(&mut self, dt: f32) {
        let actions = std::mem::take(&mut self.body_actions);
        let Some(body) = &self.scene.body else {
            return;
        };
        let rig = &body.rig;
        let n = self.game.players.len();
        self.bodies.resize_with(n, BodyAnimator::default);
        self.body_poses.resize_with(n, || None);
        for (i, p) in self.game.players.iter().enumerate() {
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
                // Shots are overlays, not drawn yet.
                if what != "fire_1" {
                    self.bodies[i].act(rig, &input, what);
                }
            }
            let pose = self.bodies[i].update(rig, &input, dt);
            let first_person = i == self.me && self.walking && p.alive;
            self.body_poses[i] = (!first_person && i < scene::MAX_BODIES).then(|| {
                let world = rig.world(&pose);
                BodyPose {
                    skin: rig.skin_matrices(&world),
                    object: Mat4::from_translation(p.body.position) * Mat4::from_rotation_z(p.yaw),
                    weapon: rig.weapon_frame(&world),
                }
            });
        }
    }

    fn message(&mut self, text: String) {
        println!("{}", text.to_lowercase());
        self.messages.push((text, 5.0));
        if self.messages.len() > 4 {
            self.messages.remove(0);
        }
    }

    /// Pick and advance the first person animation: bring a new weapon up,
    /// fire, melee, throw and reload on cue, otherwise idle.
    fn animate_view_model(&mut self, dt: f32, view: ViewEvents) {
        let weapon = self.me().held().map(|h| h.weapon);
        let Some(rig) = weapon
            .and_then(|w| self.scene.weapons.get(w))
            .and_then(|w| w.rig.as_ref())
        else {
            self.shown_weapon = weapon;
            return;
        };
        if view.switched || weapon != self.shown_weapon {
            self.shown_weapon = weapon;
            let ready = rig.find("first_person:ready", 0);
            if let Some(a) = ready.and_then(|i| rig.graph.animations.get(i)) {
                self.game.players[self.me].readying = a.duration();
            }
            self.animator.play(ready, false);
        }
        if view.fired {
            self.shots_fired += 1;
            let anim = rig.find("first_person:fire_1", self.shots_fired);
            if anim.is_some() {
                self.animator.play(anim, false);
            }
        }
        if view.melee {
            self.shots_fired += 1;
            let anim = melee_strike(rig, self.shots_fired);
            if anim.is_some() {
                self.animator.play(anim, false);
            }
        }
        if view.thrown {
            let anim = rig.find("first_person:throw_grenade", 0);
            if anim.is_some() {
                self.animator.play(anim, false);
            }
        }
        if let Some(empty) = view.reload {
            let name = if empty {
                "first_person:reload_empty"
            } else {
                "first_person:reload_full"
            };
            let anim = rig
                .find(name, 0)
                .or_else(|| rig.find("first_person:reload_full", 0));
            self.animator.play(anim, false);
        }
        if self.animator.finished(rig) {
            self.animator.play(rig.find("first_person:idle", 0), true);
        }
        self.animator.update(rig, dt);
    }

    /// The camera's frame (x forward, y left, z up, like Halo's first person
    /// models), swaying a little while walking.
    fn view_frame(&self) -> Mat4 {
        let (f, r, u) = self.camera.basis();
        let body = &self.me().body;
        let speed = (body.velocity.truncate().length() / 2.25).min(1.0);
        let bob = if self.walking && body.grounded {
            speed
        } else {
            0.0
        };
        let sway = Vec3::new(
            0.0,
            self.bob_phase.sin() * 0.004 * bob,
            -self.bob_phase.cos().abs() * 0.003 * bob,
        );
        Mat4::from_cols(
            f.extend(0.0),
            (-r).extend(0.0),
            u.extend(0.0),
            self.camera.position.extend(1.0),
        ) * Mat4::from_translation(sway)
    }

    /// Where an unanimated first person weapon sits.
    fn view_model_matrix(&self, weapon: &WeaponAssets, state: &WeaponState) -> Mat4 {
        let def = &weapon.def;
        // Without animations every gun holds its root node at the same spot:
        // low on the right, a little ahead of the eye.
        let mut offset = Vec3::new(0.105, -0.048, -0.068) - Vec3::from(weapon.grip);
        // Kick back after each shot.
        offset.x -= 0.012 * (-state.since_shot * 18.0).exp();
        // Dip while reloading and while bringing the weapon up.
        let mut dip = 0.0;
        if let Some(left) = state.reloading {
            let t = 1.0 - left / def.reload_time.max(0.01);
            dip = smoothstep(t * 4.0).min(smoothstep((1.0 - t) * 4.0));
        }
        dip = dip.max(self.me().readying / READY_TIME);
        offset.z -= dip * 0.05;
        self.view_frame()
            * Mat4::from_translation(offset)
            * Mat4::from_rotation_z(0.04)
            * Mat4::from_rotation_y(dip * 0.6)
    }

    fn build_hud(&self, w: f32, h: f32) -> Vec<gpu::HudBatch> {
        let mut hb = HudBuilder::new(w, h);
        let scene = &self.scene;
        let font = scene.hud_font;
        let me = self.me();
        let s = hb.scale();
        let shield = me.shield / self.game.rules.shield.max(1.0);
        // The shield meter flashes red while the shields are down.
        let flash = shield < 0.25 && me.alive && (self.game.time * 4.0).fract() < 0.5;
        let mut drew_tracker = false;
        for widget in &scene.player_hud {
            match widget.name.as_str() {
                "motion_tracker_background" if !drew_tracker => {
                    drew_tracker = true;
                    hb.widget(widget, hud::BLUE, hud_mode::CHANNELS, 0.0);
                }
                "shield_meter" => {
                    let color = if flash { hud::RED } else { hud::BLUE };
                    hb.widget(widget, color, hud_mode::METER_GREY, shield);
                }
                "shield_mask" => hb.widget(widget, hud::BLUE, hud_mode::PLAIN, 0.0),
                "frag_grenade_default" => {
                    hb.widget(widget, hud::BLUE, hud_mode::CHANNELS, 0.0);
                    // Plasma grenades in the left box, frags in the right;
                    // the selected kind is bright.
                    let rect = hb.widget_rect(widget);
                    let th = (rect[3] - rect[1]) * 0.5;
                    for (at, count, kind) in [
                        (0.18, me.plasmas, GrenadeKind::Plasma),
                        (0.69, me.frags, GrenadeKind::Frag),
                    ] {
                        let color = if kind == me.grenade {
                            hud::BLUE
                        } else {
                            hud::DIM_BLUE
                        };
                        hb.text(
                            font,
                            [rect[0] + (rect[2] - rect[0]) * at, rect[1] + th * 0.5],
                            th,
                            &count.to_string(),
                            color,
                        );
                    }
                }
                _ => {}
            }
        }
        // Kill feed above the motion tracker; score bottom right.
        let line = 11.0 * s;
        for (i, (text, left)) in self.messages.iter().rev().enumerate() {
            let mut color = hud::BLUE;
            color[3] *= left.min(1.0);
            hb.text_left(
                font,
                [24.0 * s, h - 140.0 * s - i as f32 * line],
                8.0 * s,
                text,
                color,
            );
        }
        // Your score, and the best of everyone else's under it.
        let best_other = (0..self.game.players.len())
            .filter(|&i| i != self.me)
            .map(|i| self.game.players[i].kills)
            .max();
        hb.text(
            font,
            [w - 40.0 * s, h - 52.0 * s],
            14.0 * s,
            &me.kills.to_string(),
            hud::BLUE,
        );
        if let Some(k) = best_other {
            hb.text(
                font,
                [w - 40.0 * s, h - 34.0 * s],
                10.0 * s,
                &k.to_string(),
                hud::DIM_BLUE,
            );
        }
        if let Some(winner) = self.game.winner {
            let text = if winner == self.me {
                "YOU WIN".to_string()
            } else {
                format!("{} WINS", player_name(self.me, winner))
            };
            hb.text(font, [w * 0.5, h * 0.3], 20.0 * s, &text, hud::BLUE);
        }
        if !me.alive {
            let text = format!("RESPAWN IN {}", me.respawn_in.ceil().max(1.0));
            hb.text(
                font,
                [w * 0.5, h * 0.5 - 6.0 * s],
                12.0 * s,
                &text,
                hud::BLUE,
            );
            return hb.finish();
        }
        if let Some(weapon) = self.game.swap_prompt(self.me) {
            if let Some(a) = scene.weapons.get(weapon) {
                let text = format!("HOLD E TO PICK UP {}", display_name(&a.def.name));
                hb.text(
                    font,
                    [w * 0.5, h * 0.5 + 64.0 * s],
                    9.0 * s,
                    &text,
                    hud::BLUE,
                );
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
                            font,
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
        if def.uses_ammo() && state.loaded == 0 && state.reloading.is_none() {
            let msg = if state.reserve == 0 {
                "NO AMMO"
            } else {
                "RELOAD"
            };
            hb.text(font, [w * 0.5, h * 0.5 + 48.0 * s], 12.0 * s, msg, hud::RED);
        } else if me.readying > 0.0 {
            hb.text(
                font,
                [w * 0.5, h * 0.5 + 48.0 * s],
                10.0 * s,
                &display_name(&def.name),
                hud::BLUE,
            );
        }
        hb.finish()
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

    fn render(&mut self) {
        let Some(g) = &self.gpu else { return };
        let (w, h) = g.size();
        let aspect = g.aspect();
        let magnification = self.magnification();
        let view_proj = self.camera.view_proj(aspect, magnification);
        let (_, r, u) = self.camera.basis();
        let mut world = self.world_draws();
        let mut posed = Vec::new();
        if let Some(body) = &self.scene.body {
            for (i, pose) in self.body_poses.iter().enumerate() {
                let Some(pose) = pose else { continue };
                let p = &self.game.players[i];
                let light = self
                    .scene
                    .level_light
                    .at(&self.scene.textures, p.body.position + Vec3::Z * 0.2);
                posed.push((body.meshes[i], body.rig.skin.pose(&pose.skin)));
                world.push(DrawCall {
                    mesh: body.meshes[i],
                    model: pose.object,
                    light,
                    colors: Some(armor_colors(i)),
                });
                let weapon = p.held().filter(|_| p.alive);
                if let Some(mesh) = weapon
                    .and_then(|h| self.scene.weapons.get(h.weapon))
                    .and_then(|w| w.world_mesh)
                {
                    world.push(DrawCall {
                        mesh,
                        model: pose.object * pose.weapon,
                        light,
                        colors: None,
                    });
                }
            }
        }
        // The first person gun and arms take the light where the player stands.
        let held_light = self
            .scene
            .level_light
            .at(&self.scene.textures, self.camera.position);
        let sprites = self.effects.sprites(r, u);
        let mut view_models = Vec::new();
        let mut view_sprites = Vec::new();
        let alive = self.me().alive;
        if let Some((weapon, state)) = self.current().filter(|_| alive) {
            if let (Some(mesh), 0) = (weapon.view_mesh, state.zoom) {
                let (model, muzzle_frame) = match (&weapon.rig, &self.scene.arms) {
                    (Some(rig), Some(arms)) if !self.animator.pose().is_empty() => {
                        // Arms and gun posed by the animation, in camera space.
                        let world = rig.world(self.animator.pose());
                        let frame = self.view_frame();
                        posed.push((
                            arms.mesh,
                            arms.skin.pose(&rig.arms_skin(&world, &arms.skeleton)),
                        ));
                        posed.push((
                            mesh,
                            weapon.skin.pose(&rig.gun_skin(&world, &weapon.skeleton)),
                        ));
                        view_models.push(DrawCall {
                            mesh: arms.mesh,
                            model: frame,
                            light: held_light,
                            colors: Some(armor_colors(self.me)),
                        });
                        let muzzle = rig
                            .gun_node_world(&world, weapon.muzzle_node)
                            .map_or(frame, |m| frame * m);
                        (frame, muzzle)
                    }
                    _ => {
                        let m = self.view_model_matrix(weapon, state);
                        let node = weapon
                            .skeleton
                            .inverse_bind
                            .get(weapon.muzzle_node)
                            .map_or(Mat4::IDENTITY, Mat4::inverse);
                        (m, m * node)
                    }
                };
                view_models.push(DrawCall {
                    mesh,
                    model,
                    light: held_light,
                    colors: None,
                });
                if state.since_shot < 0.05 {
                    let muzzle = muzzle_frame.transform_point3(Vec3::from(weapon.muzzle));
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
        let sky_view =
            glam::camera::rh::view::look_to_mat4(Vec3::ZERO, self.camera.forward(), Vec3::Z);
        let frame = Frame {
            sky: self.scene.sky.map(|mesh| DrawCall {
                mesh,
                model: Mat4::IDENTITY,
                light: None,
                colors: None,
            }),
            sky_proj: camera::projection(aspect, magnification, 1.0, 10000.0) * sky_view,
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
            for (mesh, vertices) in &posed {
                g.update_mesh(*mesh, vertices);
            }
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
                        self.taps.fire |= down;
                    }
                    MouseButton::Right => {
                        self.zoom_held = down && self.captured;
                        self.taps.zoom |= self.zoom_held;
                    }
                    MouseButton::Middle => self.taps.throw_grenade |= down && self.captured,
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { .. } if self.captured => self.taps.switch_weapon = true,
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
                    let p = &mut self.game.players[self.me];
                    p.body.position =
                        self.camera.position - Vec3::Z * p.body.biped.standing_camera_height;
                    p.body.velocity = Vec3::ZERO;
                    self.eyes = (p.eye(), p.eye());
                }
            }
            KeyCode::Escape => {
                if self.captured {
                    self.set_capture(false);
                } else {
                    event_loop.exit();
                }
            }
            KeyCode::KeyQ => self.taps.switch_weapon = true,
            KeyCode::KeyF => self.taps.melee = true,
            KeyCode::KeyR => self.taps.reload = true,
            KeyCode::KeyG => self.taps.throw_grenade = true,
            KeyCode::KeyX => self.taps.switch_grenade = true,
            KeyCode::KeyB => self.add_bot(),
            KeyCode::Digit1
            | KeyCode::Digit2
            | KeyCode::Digit3
            | KeyCode::Digit4
            | KeyCode::Digit5
            | KeyCode::Digit6
            | KeyCode::Digit7
            | KeyCode::Digit8
            | KeyCode::Digit9 => self.give_weapon(code as usize - KeyCode::Digit1 as usize),
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
    let p = &game.players[me];
    let camera = if walking {
        let eye = p.eye();
        FlyCamera::looking_at(eye, eye + glam::vec3(p.yaw.cos(), p.yaw.sin(), 0.0))
    } else {
        FlyCamera::looking_at(
            focus + glam::vec3(radius * 0.6, -radius * 0.6, radius * 0.4),
            focus,
        )
    };
    let eyes = (p.eye(), p.eye());
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
        me,
        bots: Vec::new(),
        nav,
        walking,
        title: format!(
            "Halo 2 Rust: {name} (click to play, WASD move, mouse fire/zoom, G grenade, E pick up, F melee, R reload, Q switch weapon, Esc release)"
        ),
        window: None,
        gpu: None,
        camera,
        keys: HashSet::new(),
        captured: false,
        last_frame: Instant::now(),
        fire_held: false,
        zoom_held: false,
        taps: Taps::default(),
        pending: 0.0,
        eyes,
        effects: Effects::new(),
        bob_phase: 0.0,
        animator: rig::Animator::default(),
        shown_weapon: None,
        shots_fired: 0,
        messages: Vec::new(),
        bodies: Vec::new(),
        body_actions: Vec::new(),
        body_poses: Vec::new(),
    };
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
        app.give_weapon(n);
    }
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

    #[test]
    fn kill_feed_wording() {
        assert_eq!(kill_message(0, Some(0), 1), "YOU KILLED PLAYER 2");
        assert_eq!(kill_message(0, Some(1), 0), "PLAYER 2 KILLED YOU");
        assert_eq!(kill_message(0, Some(0), 0), "YOU KILLED YOURSELF");
        assert_eq!(kill_message(0, None, 2), "PLAYER 3 DIED");
    }
}
