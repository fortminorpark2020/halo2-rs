//! A player at this computer: their camera, first person weapon, HUD and
//! controls. Splitscreen is several of these, each drawn in its own part of
//! the window.

use crate::camera::{self, FlyCamera};
use crate::effects;
use crate::feedback::ViewFeedback;
use crate::gpu::{self, hud_mode, DrawCall, Fx, SpriteVertex};
use crate::hud::{self, HudBuilder};
use crate::input::{PadPress, PadState};
use crate::rig;
use crate::scene::{Scene, Vertex, WeaponAssets};
use blam_cache::hud::{Anchor, ScreenSplit};
use gilrs::GamepadId;
use glam::{Mat4, Vec3};
use h2sim::game::{GrenadeKind, Look, Spartan, VehicleAction, TICK};
use h2sim::vehicle::{SeatDef, SeatRole};
use h2sim::{Command, Game, GameType, WeaponState, World};
use std::collections::HashSet;
use winit::keyboard::KeyCode;

/// Seconds to bring a weapon up after switching to it.
const READY_TIME: f32 = 0.5;

/// What a weapon HUD widget shows, from its name in the HUD tag. Dual-wield
/// weapons have `_left` and `_right` versions; held alone they use `_right`.
#[derive(Debug, PartialEq)]
enum HudRole {
    Background,
    AmmoMeter,
    ZoomedAmmoMeter,
    /// Always drawn as is (heat and battery frames).
    Static,
    /// The crosshair: red over an enemy in range.
    Reticle,
    Scope,
    /// Only while zoomed (scope ticks, magnification label).
    Zoomed,
    Hidden,
}

/// The role of a widget of the left hand's gun (dual wielding): only its
/// own background and ammo meter, top left.
fn left_hud_role(name: &str) -> HudRole {
    match name {
        "weapon_background_left" => HudRole::Background,
        "ammo_meter_left" => HudRole::AmmoMeter,
        _ => HudRole::Hidden,
    }
}

fn hud_role(name: &str, magnification: f32) -> HudRole {
    match name {
        "weapon_background_single" | "weapon_background_right" => HudRole::Background,
        "ammo_meter_single" | "ammo_meter_right" => HudRole::AmmoMeter,
        "ammo_meter_zoomed" => HudRole::ZoomedAmmoMeter,
        "crosshair" => HudRole::Reticle,
        "heat_background" | "heat_background_right" | "battery_meter" => HudRole::Static,
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

/// A first person animation, Halo 2's dual wield version when holding two
/// guns (`first_person:dual:idle`), else the usual one.
fn hand_animation(rig: &rig::FirstPersonRig, dual: bool, what: &str, pick: usize) -> Option<usize> {
    dual.then(|| rig.find(&format!("first_person:dual:{what}"), pick))
        .flatten()
        .or_else(|| rig.find(&format!("first_person:{what}"), pick))
}

/// One hand's first person arm and gun: its animation, pose, meshes
/// (arms, gun) and frame.
struct Hand<'a> {
    rig: &'a rig::FirstPersonRig,
    pose: &'a [rig::NodePose],
    arms: &'a crate::scene::Arms,
    meshes: (usize, usize),
    frame: Mat4,
}

/// Seconds the flash sprite shows at the muzzle after a shot (the
/// remake's own; the firing effects' particles aren't drawn).
const FLASH_TIME: f32 = 0.05;

/// A flash at the muzzle just after a shot, the gun's colour; `muzzle_frame`
/// places the gun node the muzzle hangs from.
pub fn muzzle_flash(
    out: &mut Vec<SpriteVertex>,
    weapon: &WeaponAssets,
    state: &WeaponState,
    muzzle: Vec3,
    (r, u): (Vec3, Vec3),
    scale: f32,
) {
    if state.since_shot < FLASH_TIME {
        let size = (0.012 + 0.006 * (state.since_shot * 300.0).sin().abs()) * scale;
        let [cr, cg, cb] = effects::linear(weapon.flash_color);
        effects::quad(out, muzzle, r * size, u * size, [cr, cg, cb, 0.9]);
    }
}

/// How a gun's light brightens the first person view just after a shot,
/// fading over the barrel's illumination recovery time. How bright isn't
/// in the tags read here: the flash's colour at full strength, an
/// estimate.
fn muzzle_light(
    light: Option<[f32; 3]>,
    weapon: &WeaponAssets,
    state: &WeaponState,
) -> Option<[f32; 3]> {
    let recovery = weapon.illumination_recovery;
    let left = match recovery > 0.0 {
        true => (1.0 - state.since_shot / recovery).clamp(0.0, 1.0),
        false => 0.0,
    };
    light.map(|l| std::array::from_fn(|i| l[i] + weapon.flash_color[i] * left))
}

/// Where a reload one shell at a time (the Shotgun's) has got to in the
/// first person animations: going in, then a shell after another.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ShellReload {
    #[default]
    Off,
    Enter,
    Shells,
}

/// What one hand's first person animation should react to.
struct HandCues {
    ready: bool,
    fired: bool,
    melee: bool,
    thrown: bool,
    reload: Option<bool>,
    /// The gun is reloading now.
    reloading: bool,
}

fn animate_hand(
    animator: &mut rig::Animator,
    rig: &rig::FirstPersonRig,
    dual: bool,
    cues: HandCues,
    (shots, shells): (&mut usize, &mut ShellReload),
    dt: f32,
) {
    if cues.ready {
        animator.play(hand_animation(rig, dual, "ready", *shots), false);
        *shells = ShellReload::Off;
    }
    // A reload a shell at a time: going in, a shell after another while
    // the gun reloads, and out again once it's done (or stopped to fire).
    match *shells {
        ShellReload::Enter | ShellReload::Shells if !cues.reloading => {
            *shells = ShellReload::Off;
            if !cues.fired {
                animator.play(rig.find("first_person:reload_exit", 0), false);
            }
        }
        ShellReload::Enter if animator.finished(rig) => {
            *shells = ShellReload::Shells;
            animator.play(rig.find("first_person:reload_continue_empty", 0), true);
        }
        _ => {}
    }
    if cues.fired {
        *shots += 1;
        *shells = ShellReload::Off;
        let anim = hand_animation(rig, dual, "fire_1", *shots);
        if anim.is_some() {
            animator.play(anim, false);
        }
    }
    if (cues.melee || cues.thrown) && !dual {
        *shells = ShellReload::Off;
    }
    if cues.melee && !dual {
        *shots += 1;
        let anim = melee_strike(rig, *shots);
        if anim.is_some() {
            animator.play(anim, false);
        }
    }
    if cues.thrown && !dual {
        let anim = rig.find("first_person:throw_grenade", 0);
        if anim.is_some() {
            animator.play(anim, false);
        }
    }
    let enter = rig.find("first_person:reload_enter", 0);
    if let (Some(_), Some(enter)) = (cues.reload, enter) {
        animator.play(Some(enter), false);
        *shells = ShellReload::Enter;
    } else if let Some(empty) = cues.reload {
        let name = if empty && !dual {
            "reload_empty"
        } else {
            "reload_full"
        };
        let anim =
            hand_animation(rig, dual, name, 0).or_else(|| rig.find("first_person:reload_full", 0));
        animator.play(anim, false);
    }
    if animator.finished(rig) {
        animator.play(hand_animation(rig, dual, "idle", *shots), true);
    }
    animator.update(rig, dt);
}

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// A look's primary and secondary armour colours.
pub fn armor_colors(look: Look) -> [[f32; 3]; 2] {
    look.colors
        .map(|c| crate::profile::COLORS[c as usize % crate::profile::COLORS.len()].1)
}

/// How much of the motion sensor's art its range spans, the colour of
/// teammates on it, and its pulses per second.
const SENSOR_FILL: f32 = 0.82;
const SENSOR_ALLY: [f32; 4] = [1.0, 0.85, 0.25, 0.95];
const SENSOR_PULSE: f32 = 1.0;

/// Red and blue team armour (the game's multiplayer globals).
pub const TEAM_COLORS: [[f32; 3]; 2] = [[0.757, 0.243, 0.243], [0.212, 0.224, 0.788]];
pub const TEAM_NAMES: [&str; 2] = ["RED", "BLUE"];

/// A team's colour, bright enough for text and markers.
pub fn team_hud_color(team: u8) -> [f32; 4] {
    [[1.0, 0.3, 0.25, 1.0], [0.35, 0.55, 1.0, 1.0]][team.min(1) as usize]
}

/// Armour colours in this game: their own, or their team's in team games
/// (the campaign has no team colours).
pub fn player_colors(game: &Game, player: usize) -> [[f32; 3]; 2] {
    let campaign = game.rules.game_type == GameType::Campaign;
    match game.players.get(player) {
        Some(p) if game.rules.game_type.teams() && !campaign => {
            let c = TEAM_COLORS[p.team.min(1) as usize];
            [c, c.map(|v| v * 0.6)]
        }
        Some(p) => armor_colors(p.look),
        None => armor_colors(Look::default()),
    }
}

/// The shield meter's overshield layers.
const OVERSHIELD_GREEN: [f32; 4] = [0.35, 1.0, 0.45, 0.95];
const OVERSHIELD_YELLOW: [f32; 4] = [1.0, 0.9, 0.3, 0.95];

/// How a player's power-ups show on them: see-through with active
/// camouflage, glowing with an overshield.
pub fn player_fx(game: &Game, player: usize) -> Fx {
    match game.players.get(player) {
        Some(p) if p.alive => Fx {
            camo: 1.0 - p.visibility(),
            overshield: p.overshield(&game.rules),
            ..Fx::default()
        },
        _ => Fx::default(),
    }
}

/// Your own arms and gun with active camouflage: still faintly there so
/// you can see what you hold. (An overshield shows on the HUD instead.)
/// Your shields flaring from a hit show on your arms, in the colour Halo 2
/// gives them in first person.
fn own_fx(game: &Game, player: usize, flare: f32, color: [f32; 3]) -> Fx {
    Fx {
        camo: player_fx(game, player).camo * 0.8,
        overshield: 0.0,
        shield: flare,
        shield_color: color,
    }
}

/// A score as shown: points, or in timed games minutes and seconds.
pub fn score_text(score: i32, timed: bool) -> String {
    if timed {
        let sign = if score < 0 { "-" } else { "" };
        let t = score.unsigned_abs();
        format!("{sign}{}:{:02}", t / 60, t % 60)
    } else {
        score.to_string()
    }
}

/// "battle_rifle" -> "BATTLE RIFLE".
pub fn display_name(name: &str) -> String {
    name.replace('_', " ").to_uppercase()
}

/// Player `i` as player `me` is told about them: "YOU", or their gamertag.
pub fn player_name(game: &Game, me: usize, i: usize) -> String {
    match game.name(i) {
        _ if i == me => "YOU".into(),
        "" => format!("PLAYER {}", i + 1),
        name => name.into(),
    }
}

/// A kill feed line, as Halo 2 words it. A betrayal is a teammate's kill.
pub fn kill_message(
    game: &Game,
    me: usize,
    killer: Option<usize>,
    victim: usize,
    betrayal: bool,
) -> String {
    let name = |i| player_name(game, me, i);
    let v = name(victim);
    match killer {
        Some(k) if k == victim && k == me => "YOU KILLED YOURSELF".into(),
        Some(k) if k == victim => format!("{v} COMMITTED SUICIDE"),
        Some(k) if betrayal => format!("{} BETRAYED {v}", name(k)),
        Some(k) => format!("{} KILLED {v}", name(k)),
        None => format!("{v} DIED"),
    }
}

/// Presses since the last game tick, so a quick click between ticks still counts.
#[derive(Default)]
pub struct Taps {
    pub fire: bool,
    pub zoom: bool,
    pub melee: bool,
    pub reload: bool,
    pub switch_weapon: bool,
    pub throw_grenade: bool,
    pub switch_grenade: bool,
    pub vision: bool,
}

/// What a player's first person view should react to this frame.
#[derive(Default)]
pub struct ViewEvents {
    pub fired: bool,
    pub fired_left: bool,
    pub melee: bool,
    pub thrown: bool,
    pub reload: Option<bool>,
    pub reload_left: Option<bool>,
    pub switched: bool,
}

/// Keyboard and mouse state, for the player using them.
pub struct Keyboard<'a> {
    pub keys: &'a HashSet<KeyCode>,
    pub captured: bool,
    pub fire_held: bool,
    pub zoom_held: bool,
}

/// What one player's view needs drawn on top of the shared world.
pub struct ViewDraws {
    pub view_models: Vec<DrawCall>,
    pub view_sprites: Vec<SpriteVertex>,
    pub posed: Vec<(usize, Vec<Vertex>)>,
}

/// The seat player `i` rides in: vehicle, seat and the seat's kind.
pub fn seat_of(game: &Game, i: usize) -> Option<(usize, usize, &SeatDef)> {
    let p = game.players.get(i).filter(|p| p.alive)?;
    let (v, s) = p.seat?;
    let veh = game.vehicles.get(v)?;
    Some((v, s, game.vehicle_defs.get(veh.def)?.seats.get(s)?))
}

/// Where player `i` sees from (see `Game::view_point`).
pub fn view_point(game: &Game, i: usize) -> Vec3 {
    game.view_point(i)
}

/// How far behind the view point the camera follows player `i`'s vehicle,
/// if it does.
fn chase_distance(game: &Game, i: usize) -> Option<f32> {
    game.chase_distance(i)
}

/// How far away the crosshair shows a player's name.
const NAME_RANGE: f32 = 60.0;
/// Seconds a name stays up after the crosshair leaves them.
const NAME_LINGER: f32 = 0.6;
/// Teammates this close and in sight have their names over them.
const TEAMMATE_NAME_RANGE: f32 = 25.0;

pub struct LocalPlayer {
    /// The game player this person controls.
    pub player: usize,
    /// Played with the keyboard and mouse (player one).
    pub keyboard: bool,
    pub pad: Option<GamepadId>,
    /// The controller they lost (unplugged, or out of battery): it coming
    /// back, or A on another, takes them back.
    pub lost_pad: Option<GamepadId>,
    /// Flying freely (Tab) instead of walking.
    pub flying: bool,
    pub camera: FlyCamera,
    pub taps: Taps,
    /// Eye position after the previous and the latest tick, for smooth
    /// motion between ticks.
    pub eyes: (Vec3, Vec3),
    pub bob_phase: f32,
    /// First person arms and gun animation.
    pub animator: rig::Animator,
    /// The weapon the first person animation belongs to.
    pub shown_weapon: Option<usize>,
    pub shots_fired: usize,
    /// The left hand's gun and animation, dual wielding.
    pub left_animator: rig::Animator,
    pub shown_left: Option<usize>,
    pub left_shots: usize,
    pub shown_dual: bool,
    /// Each hand's reload a shell at a time, as the animations show it.
    pub shell_reload: [ShellReload; 2],
    /// The firing effect each hand's gun used last (its muzzle flashes).
    pub firing_effect: [usize; 2],
    /// Camera kicks, shakes and screen flashes.
    pub feedback: ViewFeedback,
    /// Their shields flaring from a hit, 0-1 (on their arms).
    pub shield_flare: f32,
    /// Kill feed and pickups, newest last, with seconds left on screen.
    pub messages: Vec<(String, f32)>,
    /// A standing line at the top of the view (LAN games to join).
    pub notice: Option<String>,
    pub view: ViewEvents,
    /// The player last under the crosshair, and seconds their name stays up.
    pub tagged: Option<(usize, f32)>,
    /// Teammates nearby in plain sight, who have their names over them.
    pub friends_seen: Vec<usize>,
    /// The player under the crosshair right now, and how far.
    pub aimed_at: Option<(usize, f32)>,
    /// The enemy the gun's autoaim is on, if any: the crosshair is red.
    pub autoaimed: Option<usize>,
    /// Where the mission's waypoints point.
    pub nav_points: Vec<Vec3>,
}

impl LocalPlayer {
    pub fn new(player: usize, game: &Game) -> LocalPlayer {
        let p = &game.players[player];
        let eye = p.eye();
        LocalPlayer {
            player,
            keyboard: false,
            pad: None,
            lost_pad: None,
            flying: false,
            camera: FlyCamera::looking_at(eye, eye + glam::vec3(p.yaw.cos(), p.yaw.sin(), 0.0)),
            taps: Taps::default(),
            eyes: (eye, eye),
            bob_phase: 0.0,
            animator: rig::Animator::default(),
            shown_weapon: None,
            shots_fired: 0,
            left_animator: rig::Animator::default(),
            shown_left: None,
            left_shots: 0,
            shown_dual: false,
            shell_reload: Default::default(),
            firing_effect: [0; 2],
            feedback: ViewFeedback::default(),
            shield_flare: 0.0,
            messages: Vec::new(),
            notice: None,
            view: ViewEvents::default(),
            tagged: None,
            friends_seen: Vec::new(),
            aimed_at: None,
            autoaimed: None,
            nav_points: Vec::new(),
        }
    }

    pub fn me<'a>(&self, game: &'a Game) -> &'a Spartan {
        &game.players[self.player]
    }

    /// Seen in first person: alive and walking (or riding where the
    /// rider looks out themselves).
    pub fn first_person(&self, game: &Game) -> bool {
        !self.flying && self.me(game).alive && chase_distance(game, self.player).is_none()
    }

    /// The gun fired from this player's seat (driving or gunning), and its
    /// state; `Some(None)` in a seat without one.
    pub fn seat_gun<'a>(
        &self,
        scene: &'a Scene,
        game: &'a Game,
    ) -> Option<Option<(&'a WeaponAssets, &'a WeaponState)>> {
        let (v, s, seat) = seat_of(game, self.player)?;
        if seat.role == SeatRole::Passenger {
            return None;
        }
        let gun = seat.weapon.and_then(|w| {
            let state = game.vehicles[v].weapons.get(s)?.as_ref()?;
            Some((scene.weapons.get(w)?, state))
        });
        Some(gun)
    }

    pub fn current<'a>(
        &self,
        scene: &'a Scene,
        game: &'a Game,
    ) -> Option<(&'a WeaponAssets, &'a WeaponState)> {
        let held = self.me(game).held()?;
        Some((scene.weapons.get(held.weapon)?, &held.state))
    }

    pub fn magnification(&self, scene: &Scene, game: &Game) -> f32 {
        if self.seat_gun(scene, game).is_some() {
            return 1.0;
        }
        self.current(scene, game)
            .map(|(w, s)| w.def.magnification(s.zoom))
            .unwrap_or(1.0)
    }

    /// Prompts name the controller's buttons (X, Y) rather than keys (E,
    /// Q): for players two to four, and player one once a controller's A
    /// took them over.
    pub fn pad_prompts(&self) -> bool {
        self.pad.is_some() || !self.keyboard
    }

    pub fn message(&mut self, text: String) {
        self.messages.push((text, 5.0));
        if self.messages.len() > 4 {
            self.messages.remove(0);
        }
    }

    /// This tick's controls in `game`, from the keyboard and mouse and/or a
    /// controller.
    pub fn command(
        &self,
        game: &Game,
        keyboard: Option<&Keyboard>,
        pad: Option<PadState>,
    ) -> Command {
        let mut cmd = Command {
            yaw: self.camera.yaw,
            pitch: self.camera.pitch,
            ..Command::default()
        };
        if self.flying {
            return cmd;
        }
        let t = &self.taps;
        cmd.fire = t.fire;
        cmd.zoom = t.zoom;
        cmd.reload = t.reload;
        cmd.melee = t.melee;
        cmd.switch_weapon = t.switch_weapon;
        cmd.throw_grenade = t.throw_grenade;
        cmd.switch_grenade = t.switch_grenade;
        cmd.vision = t.vision;
        if let Some(k) = keyboard {
            let held = |c: KeyCode| k.keys.contains(&c);
            let axis =
                |pos: KeyCode, neg: KeyCode| held(pos) as i32 as f32 - held(neg) as i32 as f32;
            cmd.movement = glam::vec2(
                axis(KeyCode::KeyD, KeyCode::KeyA),
                axis(KeyCode::KeyW, KeyCode::KeyS),
            );
            cmd.jump = held(KeyCode::Space);
            cmd.crouch = held(KeyCode::ControlLeft) || held(KeyCode::KeyC);
            cmd.fire = (k.fire_held || t.fire) && k.captured;
            cmd.zoom |= k.zoom_held || held(KeyCode::KeyZ);
            cmd.reload |= held(KeyCode::KeyR);
            cmd.action = held(KeyCode::KeyE);
            // Held, for taking and firing a second gun.
            cmd.switch_weapon |= held(KeyCode::KeyQ);
            cmd.throw_grenade |= held(KeyCode::KeyG) && k.captured;
        }
        if let Some(p) = pad {
            if p.left != glam::Vec2::ZERO {
                cmd.movement = p.left;
            }
            cmd.jump |= p.jump;
            cmd.crouch |= p.crouch;
            cmd.fire |= p.fire;
            // Dual wielding, the left trigger fires the left gun; clicking
            // the right stick does nothing (zoom fires it from the mouse).
            cmd.zoom |= p.zoom && !dual_wielding(game, self.player);
            cmd.action |= p.action;
            cmd.switch_weapon |= p.switch;
            cmd.throw_grenade |= p.grenade;
            // Halo 2's X both reloads and picks up.
            cmd.reload |= p.action;
        }
        cmd
    }

    /// Follow the player's eye between ticks, or watch their body while dead.
    pub fn update_camera(&mut self, game: &Game, world: &World, pending: f32, dt: f32) {
        if self.flying {
            return;
        }
        let me = self.me(game);
        if let (true, Some(dist)) = (me.alive, chase_distance(game, self.player)) {
            // Following the vehicle, kept out of walls.
            let a = (pending / TICK).clamp(0.0, 1.0);
            let at = self.eyes.0.lerp(self.eyes.1, a);
            let back = -self.camera.forward();
            let d = world
                .raycast(at, back, dist)
                .map_or(dist, |t| (t - 0.2).max(0.2));
            self.camera.position = at + back * d;
        } else if me.alive {
            let a = (pending / TICK).clamp(0.0, 1.0);
            self.camera.position = self.eyes.0.lerp(self.eyes.1, a);
            if me.body.grounded && me.seat.is_none() {
                self.bob_phase += me.body.velocity.truncate().length() * dt * 4.5;
            }
        } else {
            // Watch your body from behind until you respawn.
            let centre = me.body.position + Vec3::Z * 0.4;
            let back = -self.camera.forward();
            let dist = world
                .raycast(centre, back, 2.5)
                .map_or(2.5, |t| (t - 0.2).max(0.2));
            self.camera.position = centre + back * dist;
        }
        for m in &mut self.messages {
            m.1 -= dt;
        }
        self.messages.retain(|m| m.1 > 0.0);
        self.tag(game, world, dt);
    }

    /// Note who is under the crosshair and which teammates are in sight,
    /// so their names show over them.
    fn tag(&mut self, game: &Game, world: &World, dt: f32) {
        self.friends_seen.clear();
        self.aimed_at = None;
        self.autoaimed = None;
        let me = self.me(game);
        if !me.alive {
            self.tagged = None;
            return;
        }
        let eye = self.camera.position;
        let own = me.seat.map(|(v, _)| v);
        for (j, q) in game.players.iter().enumerate() {
            if j == self.player || !q.alive || game.is_enemy(self.player, j) {
                continue;
            }
            if own.is_some() && q.seat.map(|(v, _)| v) == own {
                continue;
            }
            let to = q.eye() - eye;
            let d = to.length();
            if d < TEAMMATE_NAME_RANGE && world.raycast(eye, to / d.max(1e-3), d).is_none() {
                self.friends_seen.push(j);
            }
        }
        let dir = self.camera.forward();
        // The gun the crosshair belongs to: the seat's, or the one in hand.
        let gun = match me.seat.and_then(|(v, s)| game.seat_weapon(v, s)) {
            Some(def) => Some((def, 0)),
            None => me
                .held()
                .and_then(|h| Some((game.weapons.get(h.weapon)?, h.state.zoom))),
        };
        self.autoaimed = gun
            .and_then(|(def, zoom)| game.autoaim(world, self.player, eye, dir, def, zoom))
            .map(|(j, _)| j);
        self.aimed_at = game.player_along(world, self.player, eye, dir, NAME_RANGE);
        match self.aimed_at {
            Some((j, _)) => self.tagged = Some((j, NAME_LINGER)),
            None => {
                if let Some(t) = &mut self.tagged {
                    t.1 -= dt;
                }
            }
        }
        self.tagged = self
            .tagged
            .filter(|&(j, left)| left > 0.0 && game.players.get(j).is_some_and(|p| p.alive));
    }

    /// The crosshair's colour: red while the gun's autoaim is on an enemy
    /// (one under it or close to it, in the weapon's range), as in Halo 2.
    fn reticle_color(&self) -> [f32; 4] {
        match self.autoaimed {
            Some(_) => hud::RED,
            None => hud::BLUE,
        }
    }

    /// Gamertags over teammates nearby and over whoever is under the
    /// crosshair, as Halo 2 shows them.
    fn name_tags(&self, hb: &mut HudBuilder, scene: &Scene, game: &Game, (w, h): (f32, f32)) {
        let me = self.me(game);
        if !me.alive {
            return;
        }
        let view_proj = self
            .camera
            .view_proj(w / h.max(1.0), self.magnification(scene, game));
        let t = hb.text_scale();
        let tagged = self
            .tagged
            .map(|t| t.0)
            .filter(|j| !self.friends_seen.contains(j));
        for j in self.friends_seen.iter().copied().chain(tagged) {
            let Some(q) = game.players.get(j).filter(|q| q.alive) else {
                continue;
            };
            let above = q.body.position + Vec3::Z * (q.body.height() + 0.25);
            let clip = view_proj * above.extend(1.0);
            if clip.w <= 0.01 {
                continue;
            }
            let ndc = clip.truncate() / clip.w;
            if ndc.x.abs() > 1.0 || ndc.y.abs() > 1.0 {
                continue;
            }
            let (x, y) = ((ndc.x * 0.5 + 0.5) * w, (0.5 - ndc.y * 0.5) * h);
            let color = if game.is_enemy(self.player, j) {
                hud::RED
            } else {
                hud::BLUE
            };
            let size = 8.0 * t;
            hb.text(scene.hud_font, [x, y - size], size, &q.name, color);
        }
    }

    /// Pick and advance the first person animations: bring a new weapon up,
    /// fire, melee, throw and reload on cue, otherwise idle. Dual wielding,
    /// each hand plays Halo 2's dual animations on its own gun.
    pub fn animate_view_model(&mut self, scene: &Scene, game: &Game, dt: f32) {
        let view = std::mem::take(&mut self.view);
        let me = self.me(game);
        let left = me.left.as_ref().map(|h| h.weapon);
        let dual = left.is_some();
        let switched = view.switched || dual != self.shown_dual;
        self.shown_dual = dual;
        let elite = me.look.elite;
        let rig_of = |w: Option<usize>| {
            w.and_then(|w| scene.weapons.get(w))
                .and_then(|w| w.rig_for(elite))
        };
        let right = me.held().map(|h| h.weapon);
        let reloading =
            [me.held(), me.left.as_ref()].map(|h| h.is_some_and(|h| h.state.reloading.is_some()));
        if let Some(rig) = rig_of(right) {
            let hand = HandCues {
                ready: switched || right != self.shown_weapon,
                fired: view.fired,
                melee: view.melee,
                thrown: view.thrown,
                reload: view.reload,
                reloading: reloading[0],
            };
            animate_hand(
                &mut self.animator,
                rig,
                dual,
                hand,
                (&mut self.shots_fired, &mut self.shell_reload[0]),
                dt,
            );
        }
        self.shown_weapon = right;
        if let Some(rig) = rig_of(left) {
            let hand = HandCues {
                ready: switched || left != self.shown_left,
                fired: view.fired_left,
                melee: false,
                thrown: false,
                reload: view.reload_left,
                reloading: reloading[1],
            };
            animate_hand(
                &mut self.left_animator,
                rig,
                true,
                hand,
                (&mut self.left_shots, &mut self.shell_reload[1]),
                dt,
            );
        }
        self.shown_left = left;
    }

    /// The sounds this player's first person animations started (the
    /// Shotgun's pump and shells...), in `Scene::sounds`.
    pub fn take_view_sounds(&mut self) -> Vec<usize> {
        let mut out = self.animator.take_sounds();
        out.extend(self.left_animator.take_sounds());
        out
    }

    /// The camera's frame (x forward, y left, z up, like Halo's first person
    /// models), swaying a little while walking.
    fn view_frame(&self, game: &Game) -> Mat4 {
        let (f, r, u) = self.camera.basis();
        let body = &self.me(game).body;
        let speed = (body.velocity.truncate().length() / 2.25).min(1.0);
        let bob = if !self.flying && body.grounded {
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
    fn view_model_matrix(&self, game: &Game, weapon: &WeaponAssets, state: &WeaponState) -> Mat4 {
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
        dip = dip.max(self.me(game).readying / READY_TIME);
        offset.z -= dip * 0.05;
        self.view_frame(game)
            * Mat4::from_translation(offset)
            * Mat4::from_rotation_z(0.04)
            * Mat4::from_rotation_y(dip * 0.6)
    }

    /// The first person arms and gun, and the muzzle flash.
    pub fn view_draws(&self, scene: &Scene, game: &Game) -> ViewDraws {
        let mut out = ViewDraws {
            view_models: Vec::new(),
            view_sprites: Vec::new(),
            posed: Vec::new(),
        };
        if !self.first_person(game) {
            return out;
        }
        let Some((weapon, state)) = self.current(scene, game) else {
            return out;
        };
        let (Some(mesh), 0) = (weapon.view_mesh, state.zoom) else {
            return out;
        };
        // The gun and arms take the light where the player stands, and the
        // flash of their gun's shots.
        let light = scene.level_light.at(&scene.textures, self.camera.position);
        let mut light = muzzle_light(light, weapon, state);
        let (_, r, u) = self.camera.basis();
        let elite = self.me(game).look.elite;
        let arms_fx = own_fx(
            game,
            self.player,
            self.shield_flare,
            scene.shield_colors_fp[elite as usize],
        );
        let left = self.me(game).left.as_ref();
        let left = left.and_then(|h| Some((scene.weapons.get(h.weapon)?, &h.state)));
        if let Some((lw, ls)) = left {
            light = muzzle_light(light, lw, ls);
        }
        let posed = match (weapon.rig_for(elite), scene.arms_for(elite)) {
            (Some(rig), Some(arms)) if !self.animator.pose().is_empty() => {
                let frame = self.view_frame(game);
                let hand = Hand {
                    rig,
                    pose: self.animator.pose(),
                    arms,
                    meshes: (arms.mesh, mesh),
                    frame,
                };
                let world = self.draw_hand(&mut out, game, weapon, &hand, (light, arms_fx));
                // The flag's cloth hangs from the top of the pole.
                let carried = game
                    .flags
                    .iter()
                    .find(|f| f.carrier == Some(self.player))
                    .filter(|_| game.rules.game_type == h2sim::GameType::Ctf);
                if let (Some(flag), Some(f)) = (&scene.flag, carried) {
                    let (node, at) = flag.view_attach;
                    let pole = rig
                        .gun_node_world(&world, node)
                        .map_or(frame, |m| frame * m);
                    out.view_models.push(DrawCall {
                        mesh: flag.cloth,
                        model: pole * Mat4::from_translation(at),
                        light,
                        colors: Some(crate::objective::flag_colors(f.team)),
                        emblem: crate::objective::flag_emblem(game, f.team),
                        fx: Fx::default(),
                    });
                }
                // The left hand's gun: the same arm and gun, mirrored.
                if let Some((lw, ls)) = left {
                    if let (Some(lrig), Some(lmesh)) = (lw.rig_for(elite), lw.mirror_mesh) {
                        if !self.left_animator.pose().is_empty() {
                            let hand = Hand {
                                rig: lrig,
                                pose: self.left_animator.pose(),
                                arms,
                                meshes: (arms.mirror, lmesh),
                                frame: frame * Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0)),
                            };
                            self.draw_hand(&mut out, game, lw, &hand, (light, arms_fx));
                            if let Some(at) = self.muzzle_point(scene, game, true) {
                                muzzle_flash(&mut out.view_sprites, lw, ls, at, (r, u), 1.0);
                            }
                        }
                    }
                }
                true
            }
            _ => false,
        };
        if !posed {
            out.view_models.push(DrawCall {
                mesh,
                model: self.view_model_matrix(game, weapon, state),
                light,
                colors: None,
                emblem: None,
                fx: Fx::default(),
            });
        }
        if let Some(at) = self.muzzle_point(scene, game, false) {
            muzzle_flash(&mut out.view_sprites, weapon, state, at, (r, u), 1.0);
        }
        out
    }

    /// Where the first person gun in a hand (`left`, else the right) flashes
    /// now, for its last shot's firing effect: in the world, for tracers to
    /// start from. None unless seen in first person, unzoomed.
    pub fn muzzle_point(&self, scene: &Scene, game: &Game, left: bool) -> Option<Vec3> {
        if !self.first_person(game) {
            return None;
        }
        let me = self.me(game);
        let held = match left {
            true => me.left.as_ref()?,
            false => me.held()?,
        };
        let weapon = scene.weapons.get(held.weapon)?;
        if held.state.zoom != 0 || weapon.view_mesh.is_none() {
            return None;
        }
        let muzzle = weapon.muzzle(self.firing_effect[left as usize]);
        let elite = me.look.elite;
        let animator = if left {
            &self.left_animator
        } else {
            &self.animator
        };
        let mut frame = self.view_frame(game);
        if left {
            frame *= Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0));
        }
        let node = match (weapon.rig_for(elite), scene.arms_for(elite)) {
            (Some(rig), Some(_)) if !animator.pose().is_empty() => {
                let world = rig.world(animator.pose());
                rig.gun_node_world(&world, muzzle.node)
                    .map_or(frame, |m| frame * m)
            }
            // Only the right hand's gun is drawn without animations.
            _ if left => return None,
            _ => {
                let bind = weapon
                    .skeleton
                    .inverse_bind
                    .get(muzzle.node)
                    .map_or(Mat4::IDENTITY, Mat4::inverse);
                self.view_model_matrix(game, weapon, &held.state) * bind
            }
        };
        Some(node.transform_point3(Vec3::from(muzzle.offset)))
    }

    /// Pose and draw one hand's arm and gun, in `light`, the arms with
    /// `fx`. Returns the rig's world matrices.
    fn draw_hand(
        &self,
        out: &mut ViewDraws,
        game: &Game,
        weapon: &WeaponAssets,
        hand: &Hand,
        (light, fx): (Option<[f32; 3]>, Fx),
    ) -> Vec<Mat4> {
        let arms = hand.arms;
        let world = hand.rig.world(hand.pose);
        let (arms_mesh, gun_mesh) = hand.meshes;
        out.posed.push((
            arms_mesh,
            arms.skin.pose(&hand.rig.arms_skin(&world, &arms.skeleton)),
        ));
        out.posed.push((
            gun_mesh,
            weapon
                .skin
                .pose(&hand.rig.gun_skin(&world, &weapon.skeleton)),
        ));
        out.view_models.push(DrawCall {
            mesh: arms_mesh,
            model: hand.frame,
            light,
            colors: Some(player_colors(game, self.player)),
            emblem: None,
            fx,
        });
        // The shields' flare is on the arms, not the gun.
        out.view_models.push(DrawCall {
            mesh: gun_mesh,
            model: hand.frame,
            light,
            colors: None,
            emblem: None,
            fx: Fx { shield: 0.0, ..fx },
        });
        world
    }

    /// Projection for the first person weapon, which ignores zoom.
    pub fn view_model_proj(&self, aspect: f32) -> Mat4 {
        camera::projection(aspect, 1.0, 0.005, 10.0) * self.camera.view()
    }

    /// Dots on the motion sensor in `rect` (window pixels), forward up.
    fn sensor_blips(&self, hb: &mut HudBuilder, scene: &Scene, game: &Game, rect: [f32; 4]) {
        let Some(texture) = scene.blip else {
            return;
        };
        let me = self.me(game);
        let center = [(rect[0] + rect[2]) * 0.5, (rect[1] + rect[3]) * 0.5];
        let radius = (rect[2] - rect[0]) * SENSOR_FILL * 0.5;
        let (sin, cos) = me.yaw.sin_cos();
        // Dots swell and fade with the sensor's pulse.
        let pulse = 1.0 - (game.time as f32 * SENSOR_PULSE).fract();
        for b in game.sensor_blips(self.player) {
            let ahead = b.offset.x * cos + b.offset.y * sin;
            let right = b.offset.x * sin - b.offset.y * cos;
            let k = radius / h2sim::game::SENSOR_RANGE;
            let [x, y] = [center[0] + right * k, center[1] - ahead * k];
            let size = radius * if b.vehicle { 0.24 } else { 0.15 } * (0.85 + 0.15 * pulse);
            let mut color = if b.ally { SENSOR_ALLY } else { hud::RED };
            color[3] *= 0.6 + 0.4 * pulse;
            hb.quad(
                texture,
                [x - size, y - size, x + size, y + size],
                [0.0, 0.0, 1.0, 1.0],
                color,
                hud_mode::PLAIN,
                0.0,
            );
        }
    }

    /// The HUD of this player's `w` x `h` view, in the tags' `split` layout.
    pub fn build_hud(
        &self,
        scene: &Scene,
        game: &Game,
        w: f32,
        h: f32,
        split: ScreenSplit,
    ) -> Vec<gpu::HudBatch> {
        let mut hb = HudBuilder::for_view(w, h, split);
        // Damage flashes the whole view, under the HUD.
        if let Some(color) = self.feedback.flash() {
            hb.quad(
                scene.hud_white,
                [0.0, 0.0, w, h],
                [0.0, 0.0, 1.0, 1.0],
                color,
                hud_mode::PLAIN,
                0.0,
            );
        }
        let font = scene.hud_font;
        let me = self.me(game);
        // The corner margins' scale, and the remake's own text's.
        let (s, t) = (hb.scale(), hb.text_scale());
        let shield = me.shield / game.rules.shield.max(1.0);
        // The shield meter flashes red while the shields are down.
        let flash = shield < 0.25 && me.alive && (game.time * 4.0).fract() < 0.5;
        let mut drew_tracker = false;
        // The kill feed sits above this, and above the shield meter and
        // motion tracker where they reach higher (as in a splitscreen view).
        let mut tracker_top = h - 120.0 * s;
        for widget in &scene.player_hud[split as usize] {
            match widget.name.as_str() {
                // With the motion sensor off (as in SWAT) there's no tracker.
                "motion_tracker_background" if !drew_tracker && game.rules.options.radar => {
                    drew_tracker = true;
                    hb.widget(widget, hud::BLUE, hud_mode::CHANNELS, 0.0);
                    let rect = hb.widget_rect(widget);
                    tracker_top = tracker_top.min(rect[1]);
                    self.sensor_blips(&mut hb, scene, game, rect);
                }
                "shield_meter" => {
                    tracker_top = tracker_top.min(hb.widget_rect(widget)[1]);
                    let color = if flash { hud::RED } else { hud::BLUE };
                    hb.widget(widget, color, hud_mode::METER_GREY, shield.min(1.0));
                    // An overshield fills the meter again, once green and
                    // once yellow.
                    for (layer, color) in [(1.0, OVERSHIELD_GREEN), (2.0, OVERSHIELD_YELLOW)] {
                        if shield > layer {
                            hb.widget(widget, color, hud_mode::METER_GREY, shield - layer);
                        }
                    }
                }
                "shield_mask" => hb.widget(widget, hud::BLUE, hud_mode::PLAIN, 0.0),
                // Dual wielding, the left gun's display takes its place.
                "frag_grenade_default" if me.left.is_none() => {
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
        // Kill feed above the shield meter and motion tracker; score
        // bottom right.
        let line = 11.0 * t;
        for (i, (text, left)) in self.messages.iter().rev().enumerate() {
            let mut color = hud::BLUE;
            color[3] *= left.min(1.0);
            hb.text_left(
                font,
                [24.0 * s, tracker_top - (i + 1) as f32 * line],
                8.0 * t,
                text,
                color,
            );
        }
        if let Some(notice) = &self.notice {
            hb.text(font, [w * 0.5, 64.0 * t], 8.0 * t, notice, hud::BLUE);
        }
        // Your score (your team's in team games), and the best of the
        // others under it. The campaign keeps no score.
        let campaign = game.rules.game_type == GameType::Campaign;
        let teams = game.rules.game_type.teams();
        let (mine, best_other, colors) = if teams {
            let enemy = 1 - me.team.min(1);
            (
                game.team_score(me.team),
                Some(game.team_score(enemy)),
                [team_hud_color(me.team), team_hud_color(enemy)],
            )
        } else {
            let best = (0..game.players.len())
                .filter(|&i| i != self.player)
                .map(|i| game.players[i].score)
                .max();
            (me.score, best, [hud::BLUE, hud::DIM_BLUE])
        };
        let timed = game.rules.game_type.timed();
        // In the corner, where Halo 2's scoreboard widgets are anchored.
        let [x, y] = hb.anchor(Anchor::Scoreboard);
        let x = x - 16.0 * t;
        if !campaign {
            hb.text(
                font,
                [x, y - 32.0 * t],
                14.0 * t,
                &score_text(mine, timed),
                colors[0],
            );
        }
        if let Some(k) = best_other.filter(|_| !campaign) {
            hb.text(
                font,
                [x, y - 14.0 * t],
                10.0 * t,
                &score_text(k, timed),
                colors[1],
            );
        }
        // The time left over the scores, down to 0:00 if time runs out
        // (gone once someone reaches the score).
        if let Some(left) = game.time_left().filter(|&l| l == 0.0 || !game.over()) {
            hb.text(
                font,
                [x, y - 50.0 * t],
                10.0 * t,
                &score_text(left.ceil() as i32, true),
                hud::BLUE,
            );
        }
        self.name_tags(&mut hb, scene, game, (w, h));
        self.objective_waypoints(&mut hb, scene, game, (w, h));
        if let Some(text) = self.objective_prompt(game) {
            hb.text(
                font,
                [w * 0.5, h * 0.5 + 64.0 * t],
                9.0 * t,
                &text,
                hud::BLUE,
            );
        }
        if game.over() {
            let text = match (game.winning_team, game.winner) {
                (Some(t), _) if t == me.team => "YOUR TEAM WINS".to_string(),
                (Some(t), _) => format!("{} TEAM WINS", TEAM_NAMES[t.min(1) as usize]),
                (None, Some(w)) if w == self.player => "YOU WIN".to_string(),
                (None, Some(w)) => format!("{} WINS", player_name(game, self.player, w)),
                (None, None) => "DRAW".to_string(),
            };
            hb.text(font, [w * 0.5, h * 0.3], 20.0 * t, &text, hud::BLUE);
        }
        if !me.alive {
            let text = format!("RESPAWN IN {}", me.respawn_in.ceil().max(1.0));
            hb.text(
                font,
                [w * 0.5, h * 0.5 - 6.0 * t],
                12.0 * t,
                &text,
                hud::BLUE,
            );
            return hb.finish();
        }
        let riding = me.seat.is_some();
        let prompts = [
            (game.swap_prompt(self.player), ["E", "X"], "PICK UP"),
            (game.dual_prompt(self.player), ["Q", "Y"], "DUAL WIELD"),
        ];
        let mut y = h * 0.5 + 64.0 * t;
        if let Some(text) = vehicle_prompt(scene, game, self.player, !self.pad_prompts()) {
            hb.text(font, [w * 0.5, y], 9.0 * t, &text, hud::BLUE);
            y += 12.0 * t;
        }
        if let Some(gun) = self.seat_gun(scene, game) {
            if let Some((weapon, state)) = gun {
                // A vehicle gun's HUD is its reticle (and one for aiming
                // at friends).
                let reticle = self.reticle_color();
                weapon_hud(&mut hb, scene, weapon, state, reticle, |name, _| {
                    if name.contains("friend") {
                        HudRole::Hidden
                    } else {
                        HudRole::Reticle
                    }
                });
            }
            return hb.finish();
        }
        for (weapon, buttons, what) in prompts.into_iter().filter(|_| !riding) {
            if let Some(a) = weapon.and_then(|w| scene.weapons.get(w)) {
                let button = buttons[self.pad_prompts() as usize];
                let name = display_name(&a.def.name);
                let text = format!("HOLD {button} TO {what} {name}");
                hb.text(font, [w * 0.5, y], 9.0 * t, &text, hud::BLUE);
                y += 12.0 * t;
            }
        }
        let Some((weapon, state)) = self.current(scene, game) else {
            return hb.finish();
        };
        let def = &weapon.def;
        let reticle = self.reticle_color();
        weapon_hud(&mut hb, scene, weapon, state, reticle, hud_role);
        let left = me.left.as_ref();
        if let Some((lw, ls)) = left.and_then(|h| Some((scene.weapons.get(h.weapon)?, &h.state))) {
            weapon_hud(&mut hb, scene, lw, ls, reticle, |name, _| {
                left_hud_role(name)
            });
        }
        if def.uses_ammo() && state.loaded == 0 && state.reloading.is_none() {
            let msg = if state.reserve == 0 {
                "NO AMMO"
            } else {
                "RELOAD"
            };
            hb.text(font, [w * 0.5, h * 0.5 + 48.0 * t], 12.0 * t, msg, hud::RED);
        } else if me.readying > 0.0 {
            hb.text(
                font,
                [w * 0.5, h * 0.5 + 48.0 * t],
                10.0 * t,
                &display_name(&def.name),
                hud::BLUE,
            );
        }
        hb.finish()
    }
}

/// What holding the action button would do to a vehicle nearby.
fn vehicle_prompt(scene: &Scene, game: &Game, i: usize, keyboard: bool) -> Option<String> {
    let button = if keyboard { "E" } else { "X" };
    let (v, what) = match game.vehicle_action(i)? {
        VehicleAction::Hijack { vehicle, .. } => (vehicle, "BOARD"),
        VehicleAction::Enter { vehicle, seat } => {
            let def = &game.vehicle_defs[game.vehicles[vehicle].def];
            let what = match def.seats[seat].role {
                SeatRole::Driver if def.drive == h2sim::vehicle::Drive::Fixed => "USE",
                SeatRole::Driver => "DRIVE",
                SeatRole::Gunner => "GUN",
                SeatRole::Passenger => "RIDE IN",
            };
            (vehicle, what)
        }
        VehicleAction::Flip { vehicle } => (vehicle, "FLIP"),
    };
    let name = &scene.vehicles.kinds.get(game.vehicles[v].def)?.name;
    Some(format!("HOLD {button} TO {what} {name}"))
}

/// A weapon's HUD widgets: background with spare ammo, ammo meter,
/// crosshair, scope. `role` says which widgets are this hand's.
fn weapon_hud(
    hb: &mut HudBuilder,
    scene: &Scene,
    weapon: &WeaponAssets,
    state: &WeaponState,
    reticle: [f32; 4],
    role: impl Fn(&str, f32) -> HudRole,
) {
    let font = scene.hud_font;
    let zoomed = state.zoom > 0;
    let def = &weapon.def;
    let magnification = def.magnification(state.zoom);
    let ammo_fill = if def.uses_ammo() {
        state.loaded as f32 / def.magazine_size.max(1) as f32
    } else {
        1.0
    };
    for widget in &weapon.hud[hb.split() as usize] {
        match role(&widget.name, magnification) {
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
            HudRole::Reticle => hb.widget(widget, reticle, hud_mode::CHANNELS, 0.0),
            HudRole::Zoomed if zoomed => {
                let color = if widget.name.ends_with("_crosshair") {
                    reticle
                } else {
                    hud::BLUE
                };
                hb.widget(widget, color, hud_mode::CHANNELS, 0.0)
            }
            _ => {}
        }
    }
}

/// The local player a press on a controller no one here plays with is for.
/// A: someone whose controller went, else player one at the keyboard.
/// Start: only a guest whose controller went (player one at the keyboard
/// plays on without theirs), else no one, and Start brings in a new player.
pub fn new_pad_for(locals: &[LocalPlayer], press: PadPress) -> Option<usize> {
    let lost = |l: &LocalPlayer| l.lost_pad.is_some();
    match press {
        // A guest whose controller went first: their view asks for A, and
        // player one can play on at the keyboard meanwhile.
        PadPress::Claim => locals
            .iter()
            .position(|l| lost(l) && !l.keyboard)
            .or_else(|| locals.iter().position(lost))
            .or_else(|| locals.iter().position(|l| l.keyboard && l.pad.is_none())),
        PadPress::Join => locals.iter().position(|l| lost(l) && !l.keyboard),
        _ => None,
    }
}

/// Player `i` holds a gun in each hand.
pub fn dual_wielding(game: &Game, i: usize) -> bool {
    game.players.get(i).is_some_and(|p| p.left.is_some())
}

/// Where each of `n` splitscreen views goes in a `w` x `h` window: one fills
/// it, two split it top and bottom, three give the first player the top
/// half and the others a bottom quarter each (as Halo 2 does: its HUD gives
/// the first of three a half screen layout), four take a quarter each.
pub fn viewports(n: usize, w: u32, h: u32) -> Vec<[u32; 4]> {
    let (hw, hh) = (w / 2, h / 2);
    match n {
        0 | 1 => vec![[0, 0, w, h]],
        2 => vec![[0, 0, w, hh], [0, hh, w, h - hh]],
        3 => vec![[0, 0, w, hh], [0, hh, hw, h - hh], [hw, hh, w - hw, h - hh]],
        _ => [
            [0, 0, hw, hh],
            [hw, 0, w - hw, hh],
            [0, hh, hw, h - hh],
            [hw, hh, w - hw, h - hh],
        ][..n.min(4)]
            .to_vec(),
    }
}

/// Which of the HUD tags' layouts view `k` of `n` draws, as Halo 2 picks
/// it: full screen alone, half for two players and the first of three,
/// quarter for the rest (Project Cartographer's rebuild of
/// `new_hud_get_screen_split_type`). It matches `viewports`.
pub fn screen_split(k: usize, n: usize) -> ScreenSplit {
    match n {
        0 | 1 => ScreenSplit::Full,
        2 => ScreenSplit::Half,
        3 if k == 0 => ScreenSplit::Half,
        _ => ScreenSplit::Quarter,
    }
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
    fn timed_scores_read_as_minutes() {
        assert_eq!(score_text(75, true), "1:15");
        assert_eq!(score_text(5, true), "0:05");
        assert_eq!(score_text(75, false), "75");
    }

    #[test]
    fn kill_feed_wording() {
        let mut g = h2sim::testing::game();
        for _ in 0..3 {
            g.add_player();
        }
        g.set_name(2, "Sarge");
        let kill = |me, killer, victim, betrayal| kill_message(&g, me, killer, victim, betrayal);
        assert_eq!(kill(0, Some(0), 1, false), "YOU KILLED PLAYER 2");
        assert_eq!(kill(0, Some(1), 0, false), "PLAYER 2 KILLED YOU");
        assert_eq!(kill(0, Some(0), 0, false), "YOU KILLED YOURSELF");
        assert_eq!(kill(0, None, 2, false), "SARGE DIED");
        assert_eq!(kill(0, Some(0), 2, true), "YOU BETRAYED SARGE");
        assert_eq!(kill(0, Some(1), 0, true), "PLAYER 2 BETRAYED YOU");
        assert_eq!(kill(1, Some(2), 2, false), "SARGE COMMITTED SUICIDE");
    }

    #[test]
    fn clicking_the_right_stick_dual_wielding_fires_nothing() {
        let mut game = h2sim::testing::game();
        let i = game.add_player();
        let l = LocalPlayer::new(i, &game);
        let pad = PadState {
            zoom: true,
            ..PadState::default()
        };
        assert!(l.command(&game, None, Some(pad)).zoom);
        // Zoom fires the left gun, so the click doesn't count.
        game.players[i].left = game.players[i].weapons.first().cloned();
        assert!(!l.command(&game, None, Some(pad)).zoom);
    }

    /// A controller's id, which gilrs hands out only for controllers it
    /// finds.
    fn pad(n: usize) -> GamepadId {
        // It is a usize inside (transmute checks that the sizes match).
        unsafe { std::mem::transmute::<usize, GamepadId>(n) }
    }

    #[test]
    fn start_on_a_new_controller_brings_in_a_player_while_player_one_lost_theirs() {
        let mut game = h2sim::testing::game();
        let mut locals: Vec<LocalPlayer> = (0..2)
            .map(|_| {
                let i = game.add_player();
                LocalPlayer::new(i, &game)
            })
            .collect();
        locals[0].keyboard = true;
        locals[1].pad = Some(pad(1));
        // Player one's controller went: they play on at the keyboard.
        locals[0].lost_pad = Some(pad(0));
        assert_eq!(new_pad_for(&locals, PadPress::Join), None);
        assert_eq!(new_pad_for(&locals, PadPress::Claim), Some(0));
        // A guest whose controller went is the one Start (or A) is for.
        locals[1].pad = None;
        locals[1].lost_pad = Some(pad(1));
        assert_eq!(new_pad_for(&locals, PadPress::Join), Some(1));
        assert_eq!(new_pad_for(&locals, PadPress::Claim), Some(1));
        locals[0].lost_pad = None;
        assert_eq!(new_pad_for(&locals, PadPress::Claim), Some(1));
        // No one lost theirs: A takes player one at the keyboard, unless
        // a controller already has them.
        locals[1].lost_pad = None;
        locals[1].pad = Some(pad(1));
        assert_eq!(new_pad_for(&locals, PadPress::Claim), Some(0));
        assert_eq!(new_pad_for(&locals, PadPress::Join), None);
        locals[0].pad = Some(pad(2));
        assert_eq!(new_pad_for(&locals, PadPress::Claim), None);
    }

    #[test]
    fn splitscreen_views_cover_the_window() {
        for n in 1..=4 {
            let area: u32 = viewports(n, 1280, 721).iter().map(|v| v[2] * v[3]).sum();
            assert_eq!(area, 1280 * 721, "{n} views");
            assert_eq!(viewports(n, 1280, 721).len(), n);
        }
        assert_eq!(viewports(2, 1280, 720)[1], [0, 360, 1280, 360]);
        assert_eq!(
            viewports(3, 1920, 1080),
            [[0, 0, 1920, 540], [0, 540, 960, 540], [960, 540, 960, 540]]
        );
    }

    #[test]
    fn hud_layouts_follow_the_views() {
        // A full width view is full or half screen; a half width one is
        // a quarter.
        for n in 1..=4 {
            for (k, v) in viewports(n, 1920, 1080).iter().enumerate() {
                let expected = match (v[2], v[3]) {
                    (1920, 1080) => ScreenSplit::Full,
                    (1920, _) => ScreenSplit::Half,
                    _ => ScreenSplit::Quarter,
                };
                assert_eq!(screen_split(k, n), expected, "view {k} of {n}");
            }
        }
    }
}
