//! A player at this computer: their camera, first person weapon, HUD and
//! controls. Splitscreen is several of these, each drawn in its own part of
//! the window.

use crate::camera::{self, FlyCamera, StickLook};
use crate::effects;
use crate::gpu::{self, hud_mode, DrawCall, Fx, SpriteVertex};
use crate::hud::{self, HudBuilder};
use crate::input::{Function, FunctionSet, PadButton, PadId, PadState, Pads};
use crate::rig;
use crate::scene::{HudWidget, Scene, Vertex, WeaponAssets};
use blam_cache::hud::{self as tags, Anchor, ScreenSplit};
use blam_cache::physics::PlayerControl;
use glam::{Mat4, Vec2, Vec3};
use h2sim::game::{GrenadeKind, Look, Magnet, Spartan, VehicleAction, TICK};
use h2sim::vehicle::{SeatDef, SeatRole};
use h2sim::{Command, Game, GameType, WeaponDef, WeaponState, World};
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
    /// The crosshair over a teammate: green.
    FriendlyReticle,
    Scope,
    /// Only while zoomed (scope ticks, magnification labels).
    Zoomed,
    Hidden,
}

/// The role of a widget of the left hand's gun (dual wielding): only its
/// own background and ammo meter, top left.
fn left_hud_role(w: &HudWidget) -> HudRole {
    match w.name.as_str() {
        "weapon_background_left" => HudRole::Background,
        "ammo_meter_left" => HudRole::AmmoMeter,
        _ => HudRole::Hidden,
    }
}

/// The role of a widget of the gun in hand. When it shows is up to its
/// state flags (`weapon_hud`).
fn hud_role(w: &HudWidget) -> HudRole {
    match w.name.as_str() {
        "weapon_background_single" | "weapon_background_right" | "weapon_background_zoomed" => {
            HudRole::Background
        }
        "ammo_meter_single" | "ammo_meter_right" => HudRole::AmmoMeter,
        "ammo_meter_zoomed" => HudRole::ZoomedAmmoMeter,
        "crosshair" => HudRole::Reticle,
        "crosshair_friendly" | "crosshair_friend" => HudRole::FriendlyReticle,
        "heat_background"
        | "heat_background_right"
        | "battery_meter"
        | "heat_background_zoomed"
        | "heat_border_zoomed"
        | "battery_background_zoomed"
        | "battery_border_zoomed" => HudRole::Static,
        n if n.contains("scope_mask") => HudRole::Scope,
        "left_crosshair" | "right_crosshair" | "top_crosshair" | "bottom_crosshair"
        | "distance_meter" | "covenant_2xa" | "covenant_2xb" => HudRole::Zoomed,
        // The magnification labels ("2x", the sniper rifle's "5x" and
        // "10x"), each at the zoom level its state flags name.
        _ if w.state.yes_unit & (tags::UNIT_ZOOM_LEVEL_1 | tags::UNIT_ZOOM_LEVEL_2) != 0 => {
            HudRole::Zoomed
        }
        _ => HudRole::Hidden,
    }
}

/// The unit state flags Halo 2's HUD widgets show by: one gun or two,
/// and the zoom level.
fn unit_flags(dual: bool, zoom: u32) -> u16 {
    let wield = if dual {
        tags::UNIT_DUAL_WIELDING
    } else {
        tags::UNIT_SINGLE_WIELDING
    };
    let zoom = match zoom {
        0 => tags::UNIT_UNZOOMED,
        1 => tags::UNIT_ZOOM_LEVEL_1,
        _ => tags::UNIT_ZOOM_LEVEL_2,
    };
    tags::UNIT_DEFAULT | wield | zoom
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

/// A flash at the muzzle just after a shot.
fn muzzle_flash(
    out: &mut Vec<SpriteVertex>,
    weapon: &WeaponAssets,
    state: &WeaponState,
    muzzle_frame: Mat4,
    (r, u): (Vec3, Vec3),
) {
    if state.since_shot < 0.05 {
        let muzzle = muzzle_frame.transform_point3(Vec3::from(weapon.muzzle));
        let size = 0.012 + 0.006 * (state.since_shot * 300.0).sin().abs();
        effects::quad(out, muzzle, r * size, u * size, [1.0, 0.8, 0.4, 0.9]);
    }
}

/// What one hand's first person animation should react to.
struct HandCues {
    ready: bool,
    fired: bool,
    melee: bool,
    thrown: bool,
    reload: Option<bool>,
}

fn animate_hand(
    animator: &mut rig::Animator,
    rig: &rig::FirstPersonRig,
    dual: bool,
    cues: HandCues,
    shots: &mut usize,
    dt: f32,
) {
    if cues.ready {
        animator.play(hand_animation(rig, dual, "ready", *shots), false);
    }
    if cues.fired {
        *shots += 1;
        let anim = hand_animation(rig, dual, "fire_1", *shots);
        if anim.is_some() {
            animator.play(anim, false);
        }
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
    if let Some(empty) = cues.reload {
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
        },
        _ => Fx::default(),
    }
}

/// Your own arms and gun with active camouflage: still faintly there so
/// you can see what you hold. (An overshield shows on the HUD instead.)
fn own_fx(game: &Game, player: usize) -> Fx {
    Fx {
        camo: player_fx(game, player).camo * 0.8,
        overshield: 0.0,
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
    /// A controller's buttons pressed, by what they do under its player's
    /// layout (done as the tick finds the game: dual wielding or not).
    pub pad: FunctionSet,
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
    pub pad: Option<PadId>,
    /// The controller they lost (unplugged, or out of battery): it coming
    /// back, or A on another, takes them back.
    pub lost_pad: Option<PadId>,
    /// Whose settings in the profile they play with: player one's (0),
    /// or guest `slot`'s, which follow the guest's controller from game to
    /// game.
    pub slot: usize,
    /// Their look settings and controller layouts (from the profile:
    /// player one's, or this guest's).
    pub controls: camera::Controls,
    /// Their controller's vibration.
    pub rumble: crate::rumble::Rumbler,
    /// Seconds they've moved forward without looking up or down, for
    /// Automatic Look Centering.
    pub level_time: f32,
    /// They played with the keyboard and mouse last (and not their
    /// controller): prompts name keys.
    pub typing: bool,
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
    /// Who the gun's autoaim is on, if anyone, and whether they're a
    /// teammate: the crosshair is red over an enemy, green over a friend.
    pub autoaimed: Option<(usize, bool)>,
    /// Where the mission's waypoints point.
    pub nav_points: Vec<Vec3>,
    /// The controller's look, sped up while the stick is held pegged.
    pub stick: StickLook,
    /// The enemy a controller's aim followed last frame, and which way
    /// they were (yaw, pitch).
    pub adhesion: Option<(usize, Vec2)>,
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
            slot: 0,
            controls: camera::Controls::default(),
            rumble: crate::rumble::Rumbler::default(),
            level_time: 0.0,
            typing: false,
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
            messages: Vec::new(),
            notice: None,
            view: ViewEvents::default(),
            tagged: None,
            friends_seen: Vec::new(),
            aimed_at: None,
            autoaimed: None,
            nav_points: Vec::new(),
            stick: StickLook::default(),
            adhesion: None,
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
        (self.pad.is_some() || !self.keyboard) && !(self.keyboard && self.typing)
    }

    /// A message from a mission's scripts, `HINT_FLASHLIGHT` in it naming
    /// the key or button for the flashlight (the Arbiter's camouflage).
    pub fn hint(&mut self, text: &str) {
        let button = self.prompt_button("V", Function::Flashlight);
        self.message(text.replace(HINT_FLASHLIGHT, button));
    }

    /// What a prompt says to press for `f`: the `key` at the keyboard, or
    /// the button the player's layout puts it on.
    pub fn prompt_button(&self, key: &'static str, f: Function) -> &'static str {
        if !self.pad_prompts() {
            return key;
        }
        self.controls
            .buttons
            .button(f)
            .map_or("?", PadButton::label)
    }

    /// This player's controller now, under their layouts, if it's
    /// connected and this window takes its input.
    pub fn pad_state(&self, pads: &Pads) -> Option<PadState> {
        // Back after it went, it waits for A (`lost_pad`).
        if self.lost_pad.is_some() {
            return None;
        }
        let reading = pads.reading(self.pad?)?;
        Some(reading.state(self.controls.buttons, self.controls.sticks))
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
        // Riding, the buttons are the seat's: as with one gun.
        let riding = game.players.get(self.player).is_some_and(|p| p.seat.is_some());
        let hands = Hands {
            dual: dual_wielding(game, self.player) && !riding,
            inverted: self.controls.dual_wield_inversion,
        };
        if let Some(p) = pad {
            if p.movement != Vec2::ZERO {
                cmd.movement = p.movement;
            }
            for f in p.held.iter() {
                apply(&mut cmd, f, hands, false);
            }
        }
        // And those pressed since the last tick, so a quick one counts.
        for f in t.pad.iter() {
            apply(&mut cmd, f, hands, true);
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
        self.autoaimed = self
            .gun(game)
            .and_then(|(def, zoom)| game.autoaim_target(world, self.player, eye, dir, def, zoom));
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

    /// The gun the crosshair belongs to, and how far it's zoomed, by
    /// `seat_gun`'s rule: a driver's or gunner's seat's, if it has one,
    /// else the one in hand. A Warthog's or Spectre's driver has no gun
    /// (the trigger sounds the horn), so no autoaim and no aim assist to
    /// pull at their steering.
    fn gun<'a>(&self, game: &'a Game) -> Option<(&'a WeaponDef, u32)> {
        if let Some((v, s, seat)) = seat_of(game, self.player) {
            if seat.role != SeatRole::Passenger {
                return Some((game.seat_weapon(v, s)?, 0));
            }
        }
        let held = self.me(game).held()?;
        Some((game.weapons.get(held.weapon)?, held.state.zoom))
    }

    /// Halo 2's dialog while their controller is gone (the game stands
    /// still for it, unless others play over the network): at the
    /// keyboard, Enter plays on there, as on Halo 2 PC; a guest
    /// reconnects it (or takes another) and presses A.
    fn lost_pad_dialog(&self, hb: &mut HudBuilder, scene: &Scene, (w, h): (f32, f32), t: f32) {
        if self.lost_pad.is_none() {
            return;
        }
        let line = if self.keyboard {
            "CONTROLLER DISCONNECTED. PRESS ENTER TO CONTINUE."
        } else {
            "PLEASE RECONNECT THE CONTROLLER AND PRESS A TO CONTINUE."
        };
        let (half_w, half_h) = ((w * 0.5 - 8.0).min(230.0 * t), 34.0 * t);
        let rect = [w * 0.5 - half_w, h * 0.5 - half_h, w * 0.5 + half_w, h * 0.5 + half_h];
        let back = [0.0, 0.03, 0.08, 0.88];
        hb.quad(scene.hud_white, rect, [0.0; 4], back, hud_mode::PLAIN, 0.0);
        let font = scene.hud_font;
        let title = "CONTROLLER DISCONNECTED";
        hb.text(font, [w * 0.5, h * 0.5 - 22.0 * t], 11.0 * t, title, hud::BLUE);
        hb.text(font, [w * 0.5, h * 0.5 + 6.0 * t], 7.0 * t, line, hud::BLUE);
    }

    /// Look around with a controller's look stick (the right one, but for
    /// the thumbstick layouts that move it), as Halo 2 does: the globals'
    /// look rates, look function and speed-up when pegged, scaled by the
    /// player's look sensitivity (and slower zoomed in). Its aim assist
    /// (the mouse has none, as in Halo 2 Vista) slows the look near an
    /// enemy in the gun's magnetism (friction), and has the crosshair
    /// follow the one it's on as they move across the view while either
    /// stick is moved (adhesion). How strongly comes from the globals; how
    /// adhesion follows (that share of the turn toward them since the last
    /// frame) is the remake's reading of it.
    pub fn look_with_stick(
        &mut self,
        scene: &Scene,
        game: &Game,
        world: &World,
        pad: &PadState,
        dt: f32,
    ) {
        let magnet = self.magnet(game, world);
        let magnification = self.magnification(scene, game);
        let look = (self.controls, magnification);
        let me = self.me(game);
        let walking = (me.alive && me.seat.is_none() && !self.flying)
            .then(|| me.body.velocity.length());
        let assisted = magnet.is_some();
        self.turn_with_stick(&scene.player_control, pad, look, magnet, dt);
        self.center_look(&scene.player_control, pad, walking, assisted, dt);
    }

    /// Automatic Look Centering, as Halo 2 has it: on foot, with the
    /// setting on, pushing the move stick more than half way forward (or
    /// back) without looking up or down and with no enemy in the aim
    /// assist, for longer than the globals' ticks (15), and the view
    /// levels out, faster the further it's tilted and the faster they go:
    /// each second by the globals' scale (0.5) times their speed times
    /// the tilt over a right angle (Halo 2's player control update,
    /// re-implemented from the CC0 decompilation; that its speed is world
    /// units a second and the step per second is the remake's reading).
    /// `walking` is their speed on foot (none riding or dead).
    fn center_look(
        &mut self,
        control: &PlayerControl,
        pad: &PadState,
        walking: Option<f32>,
        assisted: bool,
        dt: f32,
    ) {
        let engaged = self.controls.look_centering
            && walking.is_some()
            && pad.movement.y.abs() > 0.5
            && pad.look.y.abs() < 1e-4
            && !assisted;
        if !engaged {
            self.level_time = 0.0;
            return;
        }
        // Halo 2 counts its ticks (30 a second) up to 127.
        const HALO_TICK: f32 = 1.0 / 30.0;
        self.level_time = (self.level_time + dt).min(127.0 * HALO_TICK);
        if self.level_time <= f32::from(control.autolevel_ticks) * HALO_TICK {
            return;
        }
        let pitch = self.camera.pitch;
        let tilt = pitch.abs() * std::f32::consts::FRAC_2_PI;
        let step = control.autolevel_scale * walking.unwrap_or(0.0) * tilt * dt;
        self.camera.pitch -= pitch.signum() * step.min(pitch.abs());
    }

    /// `look_with_stick`, with the `(controls, magnification)` to look
    /// with and the enemy the aim is drawn to.
    fn turn_with_stick(
        &mut self,
        control: &PlayerControl,
        pad: &PadState,
        (controls, magnification): (camera::Controls, f32),
        magnet: Option<Magnet>,
        dt: f32,
    ) {
        let scale = controls.stick_scale() / magnification.max(1.0);
        let mut turn = self.stick.turn(control, pad.look, scale, dt);
        turn.y *= controls.pitch_sign();
        if let Some(m) = magnet {
            turn *= camera::friction(m.off, m.angle, control.magnetism_friction);
        }
        let toward = magnet.map(|m| (m.player, camera::angles_to(self.camera.position, m.centre)));
        let moving = pad.movement != Vec2::ZERO || pad.look != Vec2::ZERO;
        if let (Some((j, now)), Some((was_on, was)), true) = (toward, self.adhesion, moving) {
            if j == was_on {
                let adhesion = control.magnetism_adhesion.clamp(0.0, 1.0);
                turn += camera::angle_change(was, now) * adhesion;
            }
        }
        self.adhesion = toward;
        self.camera.turn(turn);
    }

    /// No controller look this frame (none, or a menu is up): its
    /// speed-up and adhesion start over.
    pub fn stop_stick(&mut self) {
        self.stick = StickLook::default();
        self.adhesion = None;
    }

    /// The enemy a controller's aim is drawn to (`Game::magnetism`).
    fn magnet(&self, game: &Game, world: &World) -> Option<Magnet> {
        if self.flying || !self.me(game).alive {
            return None;
        }
        let (def, zoom) = self.gun(game)?;
        let (eye, dir) = (self.camera.position, self.camera.forward());
        game.magnetism(world, self.player, eye, dir, def, zoom)
    }

    /// The crosshair's colour: red while the gun's autoaim is on an enemy
    /// (one under it or close to it, in the weapon's range), as in Halo 2.
    /// Its red is the tag's (the crosshair's "flash red" HUD shader).
    fn reticle_color(&self) -> [f32; 4] {
        match self.autoaimed {
            Some((_, false)) => hud::RETICLE_RED,
            _ => hud::BLUE,
        }
    }

    /// The extra state flags the HUD's widgets show by: an autoaim on a
    /// teammate shows the green crosshair in place of the usual one.
    fn extra_flags(&self) -> u16 {
        match self.autoaimed {
            Some((_, true)) => tags::EXTRA_AUTOAIM_FRIENDLY,
            _ => 0,
        }
    }

    /// Gamertags over teammates nearby and over whoever is under the
    /// crosshair, as Halo 2 shows them.
    fn name_tags(&self, hb: &mut HudBuilder, scene: &Scene, game: &Game, (w, h): (f32, f32)) {
        let me = self.me(game);
        if !me.alive {
            return;
        }
        let view_proj = self.camera.view_proj(
            &scene.lens(),
            w / h.max(1.0),
            self.magnification(scene, game),
        );
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
        if let Some(rig) = rig_of(right) {
            let hand = HandCues {
                ready: switched || right != self.shown_weapon,
                fired: view.fired,
                melee: view.melee,
                thrown: view.thrown,
                reload: view.reload,
            };
            animate_hand(
                &mut self.animator,
                rig,
                dual,
                hand,
                &mut self.shots_fired,
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
            };
            animate_hand(
                &mut self.left_animator,
                rig,
                true,
                hand,
                &mut self.left_shots,
                dt,
            );
        }
        self.shown_left = left;
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
        // The gun and arms take the light where the player stands.
        let light = scene.level_light.at(&scene.textures, self.camera.position);
        let (_, r, u) = self.camera.basis();
        let elite = self.me(game).look.elite;
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
                let (world, muzzle) = self.draw_hand(&mut out, game, weapon, &hand, light);
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
                let left = self.me(game).left.as_ref();
                let left = left.and_then(|h| Some((scene.weapons.get(h.weapon)?, &h.state)));
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
                            let (_, m) = self.draw_hand(&mut out, game, lw, &hand, light);
                            muzzle_flash(&mut out.view_sprites, lw, ls, m, (r, u));
                        }
                    }
                }
                Some(muzzle)
            }
            _ => None,
        };
        let muzzle_frame = posed.unwrap_or_else(|| {
            let m = self.view_model_matrix(game, weapon, state);
            out.view_models.push(DrawCall {
                mesh,
                model: m,
                light,
                colors: None,
                emblem: None,
                fx: Fx::default(),
            });
            let node = weapon
                .skeleton
                .inverse_bind
                .get(weapon.muzzle_node)
                .map_or(Mat4::IDENTITY, Mat4::inverse);
            m * node
        });
        muzzle_flash(&mut out.view_sprites, weapon, state, muzzle_frame, (r, u));
        out
    }

    /// Pose and draw one hand's arm and gun. Returns the rig's world
    /// matrices and the gun's muzzle frame.
    fn draw_hand(
        &self,
        out: &mut ViewDraws,
        game: &Game,
        weapon: &WeaponAssets,
        hand: &Hand,
        light: Option<[f32; 3]>,
    ) -> (Vec<Mat4>, Mat4) {
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
            fx: own_fx(game, self.player),
        });
        out.view_models.push(DrawCall {
            mesh: gun_mesh,
            model: hand.frame,
            light,
            colors: None,
            emblem: None,
            fx: own_fx(game, self.player),
        });
        let muzzle = hand
            .rig
            .gun_node_world(&world, weapon.muzzle_node)
            .map_or(hand.frame, |m| hand.frame * m);
        (world, muzzle)
    }

    /// Projection for the first person weapon, which ignores zoom (and
    /// takes the world's field of view, as Halo 2 does).
    pub fn view_model_proj(&self, lens: &camera::Lens, aspect: f32) -> Mat4 {
        lens.projection(aspect, 1.0, 0.005, 10.0) * self.camera.view()
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

    /// Where the HUD's lines under the crosshair start (`hb`'s top of
    /// text): as far below it as they sat when it was in the middle of the
    /// view, and lower if the reticles shown reach further.
    fn under_crosshair(&self, hb: &HudBuilder, scene: &Scene, game: &Game) -> f32 {
        let extra = self.extra_flags();
        let split = hb.split() as usize;
        let bottom = match self.seat_gun(scene, game) {
            Some(gun) => gun.and_then(|(weapon, _)| {
                let flags = (SEAT_UNIT_FLAGS, extra);
                reticle_bottom(hb, &weapon.hud[split], flags, seat_hud_role)
            }),
            None => {
                let me = self.me(game);
                let left = me
                    .left
                    .as_ref()
                    .and_then(|h| Some((scene.weapons.get(h.weapon)?, &h.state)));
                let right = self.current(scene, game).and_then(|(weapon, state)| {
                    let flags = (unit_flags(left.is_some(), state.zoom), extra);
                    reticle_bottom(hb, &weapon.hud[split], flags, hud_role)
                });
                let left = left.and_then(|(weapon, state)| {
                    let flags = (unit_flags(true, state.zoom), extra);
                    reticle_bottom(hb, &weapon.hud[split], flags, left_hud_role)
                });
                right.into_iter().chain(left).reduce(f32::max)
            }
        };
        under_reticle(hb, bottom)
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
        let mut hb = HudBuilder::for_view(w, h, split).with_crosshair(scene.lens().crosshair);
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
        self.lost_pad_dialog(&mut hb, scene, (w, h), t);
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
        // The lines under the crosshair: RELOAD (or the weapon's name as it
        // comes up), then the prompts below that.
        let under = self.under_crosshair(&hb, scene, game);
        if let Some(text) = self.objective_prompt(game) {
            hb.text(font, [w * 0.5, under + 16.0 * t], 9.0 * t, &text, hud::BLUE);
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
        let action = self.prompt_button("E", Function::Reload);
        let prompts = [
            (game.swap_prompt(self.player), action, "PICK UP"),
            (
                game.dual_prompt(self.player),
                self.prompt_button("Q", Function::SwitchWeapons),
                "DUAL WIELD",
            ),
        ];
        let mut y = under + 16.0 * t;
        if let Some(text) = vehicle_prompt(scene, game, self.player, action) {
            hb.text(font, [w * 0.5, y], 9.0 * t, &text, hud::BLUE);
            y += 12.0 * t;
        }
        if let Some(gun) = self.seat_gun(scene, game) {
            if let Some((weapon, state)) = gun {
                // A vehicle gun's HUD is its reticle (and one for aiming
                // at friends).
                let reticle = self.reticle_color();
                let flags = (SEAT_UNIT_FLAGS, self.extra_flags());
                weapon_hud(&mut hb, scene, weapon, state, reticle, flags, seat_hud_role);
            }
            return hb.finish();
        }
        for (weapon, button, what) in prompts.into_iter().filter(|_| !riding) {
            if let Some(a) = weapon.and_then(|w| scene.weapons.get(w)) {
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
        let left = me.left.as_ref();
        let flags = (unit_flags(left.is_some(), state.zoom), self.extra_flags());
        weapon_hud(&mut hb, scene, weapon, state, reticle, flags, hud_role);
        if let Some((lw, ls)) = left.and_then(|h| Some((scene.weapons.get(h.weapon)?, &h.state))) {
            let flags = (unit_flags(true, ls.zoom), flags.1);
            weapon_hud(&mut hb, scene, lw, ls, reticle, flags, left_hud_role);
        }
        if def.uses_ammo() && state.loaded == 0 && state.reloading.is_none() {
            let msg = if state.reserve == 0 {
                "NO AMMO"
            } else {
                "RELOAD"
            };
            hb.text(font, [w * 0.5, under], 12.0 * t, msg, hud::RED);
        } else if me.readying > 0.0 {
            hb.text(
                font,
                [w * 0.5, under],
                10.0 * t,
                &display_name(&def.name),
                hud::BLUE,
            );
        }
        hb.finish()
    }
}

/// What holding the action `button` would do to a vehicle nearby.
fn vehicle_prompt(scene: &Scene, game: &Game, i: usize, button: &str) -> Option<String> {
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

/// The unit state flags a vehicle gun's HUD shows by.
const SEAT_UNIT_FLAGS: u16 = tags::UNIT_DEFAULT | tags::UNIT_UNZOOMED;

/// What a vehicle gun's HUD widgets are: its reticle, and one for aiming at
/// friends.
fn seat_hud_role(w: &HudWidget) -> HudRole {
    if w.name.contains("friend") {
        HudRole::FriendlyReticle
    } else {
        HudRole::Reticle
    }
}

/// How far below the crosshair the HUD's lines under it start, in pixels
/// of a 1280x960 screen (counted as the remake's text is): where they sat
/// below a crosshair in the middle of the view.
const UNDER_CROSSHAIR: f32 = 48.0;

/// The lowest edge of the reticles among a weapon's HUD `widgets` (in
/// `hb`'s layout) that show with these state flags, `role` saying which
/// are reticles, in window pixels.
fn reticle_bottom(
    hb: &HudBuilder,
    widgets: &[HudWidget],
    (unit, extra): (u16, u16),
    role: impl Fn(&HudWidget) -> HudRole,
) -> Option<f32> {
    widgets
        .iter()
        .filter(|w| w.state.shows(unit, extra))
        .filter(|w| matches!(role(w), HudRole::Reticle | HudRole::FriendlyReticle))
        .map(|w| hb.widget_rect(w)[3])
        .reduce(f32::max)
}

/// Where the HUD's lines under the crosshair start: `UNDER_CROSSHAIR`
/// below it, or a little below the reticle's `bottom` edge if that is
/// lower.
fn under_reticle(hb: &HudBuilder, bottom: Option<f32>) -> f32 {
    let t = hb.text_scale();
    let under = hb.anchor(Anchor::Crosshair)[1] + UNDER_CROSSHAIR * t;
    bottom.map_or(under, |b| under.max(b + 4.0 * t))
}

/// A weapon's HUD widgets: background with spare ammo, ammo meter,
/// crosshair, scope. `role` says which widgets are this hand's; each
/// shows while the `(unit, extra)` state flags hold what its own state
/// asks for, as in Halo 2 (the sniper rifle's "10x" only at its second
/// zoom level, the green crosshair only over a friend).
fn weapon_hud(
    hb: &mut HudBuilder,
    scene: &Scene,
    weapon: &WeaponAssets,
    state: &WeaponState,
    reticle: [f32; 4],
    (unit, extra): (u16, u16),
    role: impl Fn(&HudWidget) -> HudRole,
) {
    let font = scene.hud_font;
    let zoomed = state.zoom > 0;
    let def = &weapon.def;
    let ammo_fill = if def.uses_ammo() {
        state.loaded as f32 / def.magazine_size.max(1) as f32
    } else {
        1.0
    };
    for widget in &weapon.hud[hb.split() as usize] {
        if !widget.state.shows(unit, extra) {
            continue;
        }
        match role(widget) {
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
            HudRole::FriendlyReticle => {
                hb.widget(widget, hud::RETICLE_GREEN, hud_mode::CHANNELS, 0.0)
            }
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
/// Where a mission's hint names the flashlight's key or button
/// (`LocalPlayer::hint`).
pub const HINT_FLASHLIGHT: &str = "{FLASHLIGHT}";

pub fn new_pad_for(locals: &[LocalPlayer], press: PadButton) -> Option<usize> {
    let lost = |l: &LocalPlayer| l.lost_pad.is_some();
    match press {
        // A guest whose controller went first: their view asks for A, and
        // player one can play on at the keyboard meanwhile.
        PadButton::A => locals
            .iter()
            .position(|l| lost(l) && !l.keyboard)
            .or_else(|| locals.iter().position(lost))
            .or_else(|| locals.iter().position(|l| l.keyboard && l.pad.is_none())),
        PadButton::Start => locals.iter().position(|l| lost(l) && !l.keyboard),
        _ => None,
    }
}

/// Player `i` holds a gun in each hand.
pub fn dual_wielding(game: &Game, i: usize) -> bool {
    game.players.get(i).is_some_and(|p| p.left.is_some())
}

/// The guns in a player's hands: two (dual wielding, on foot), and
/// whether their triggers swap then (Dual Wield Inversion).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Hands {
    dual: bool,
    inverted: bool,
}

/// Do what a controller's button does under its player's layout, as the
/// game is now (`hands`): function `f`, held, or (`pressed`) pressed since
/// the last tick.
fn apply(cmd: &mut Command, f: Function, hands: Hands, pressed: bool) {
    // The game's grenade button is the left hand's gun dual wielding; with
    // Dual Wield Inversion the right trigger fires it and the left the
    // right hand's.
    let swap = hands.dual && hands.inverted;
    let dual = hands.dual;
    match f {
        Function::RightWeapon if swap => cmd.throw_grenade = true,
        Function::RightWeapon => cmd.fire = true,
        Function::LeftWeapon if swap => cmd.fire = true,
        // The game's grenade button: a grenade, the left hand's gun dual
        // wielding, or a vehicle's boost or second weapon.
        Function::LeftWeapon => cmd.throw_grenade = true,
        // Boxer's B: no grenades dual wielding, so it melees then (its
        // left trigger fires the left gun).
        Function::ThrowGrenade if dual => cmd.melee = true,
        Function::ThrowGrenade => cmd.throw_grenade = true,
        Function::MeleeOrLeftWeapon if swap => cmd.fire = true,
        Function::MeleeOrLeftWeapon if dual => cmd.throw_grenade = true,
        Function::MeleeOrLeftWeapon | Function::Melee => cmd.melee = true,
        // Halo 2's X reloads when pressed, and held picks up: held on, it
        // doesn't reload again and again.
        Function::Reload => {
            cmd.reload |= pressed;
            cmd.action = true;
        }
        Function::SwitchWeapons => cmd.switch_weapon = true,
        Function::Jump => cmd.jump = true,
        Function::SwapGrenades => cmd.switch_grenade = true,
        Function::Flashlight => cmd.vision = true,
        // Dual wielding, zoom does nothing (the mouse's zoom fires the left
        // gun).
        Function::Zoom => cmd.zoom |= !dual,
        Function::Crouch => cmd.crouch = true,
    }
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
    use crate::input::{ButtonLayout, ButtonSet, PadReading, StickLayout};

    /// A HUD widget named `name` that says yes to the `yes_unit` flags.
    fn widget(name: &str, yes_unit: u16) -> HudWidget {
        HudWidget {
            name: name.into(),
            texture: 0,
            anchor: Anchor::Crosshair,
            flags: 0,
            state: tags::WidgetState {
                yes_unit,
                ..Default::default()
            },
            offset: [0.0; 2],
            registration: [0.0; 2],
            size: [1.0; 2],
        }
    }

    #[test]
    fn hud_roles_follow_widget_names_and_states() {
        let role = |name| hud_role(&widget(name, 0));
        assert_eq!(role("weapon_background_right"), HudRole::Background);
        assert_eq!(role("weapon_background_left"), HudRole::Hidden);
        assert_eq!(
            left_hud_role(&widget("weapon_background_left", 0)),
            HudRole::Background
        );
        assert_eq!(role("ammo_meter_single"), HudRole::AmmoMeter);
        assert_eq!(role("scope_mask"), HudRole::Scope);
        assert_eq!(role("crosshair_friendly"), HudRole::FriendlyReticle);
        assert_eq!(role("backpack"), HudRole::Hidden);
        // The sniper rifle's magnification labels, by their state flags
        // (its zoom levels are 3.5 and 9.5 times, its labels 5x and 10x).
        assert_eq!(hud_role(&widget("5x", 0x90)), HudRole::Zoomed);
        assert_eq!(hud_role(&widget("10x", 0x110)), HudRole::Zoomed);
    }

    /// The enemy a controller's aim is drawn to, at `centre`, `off`
    /// radians from the crosshair in the battle rifle's 6 degree cone.
    fn magnet_at(centre: Vec3, off: f32) -> Magnet {
        Magnet {
            player: 1,
            off,
            angle: 6f32.to_radians(),
            centre,
        }
    }

    #[test]
    fn friction_slows_the_stick_on_an_enemy() {
        let mut game = h2sim::testing::game();
        let i = game.add_player();
        let control = PlayerControl::default();
        let pad = PadState {
            look: Vec2::new(0.6, 0.0),
            ..PadState::default()
        };
        let look = (camera::Controls::default(), 1.0);
        let turned = |magnet| {
            let mut l = LocalPlayer::new(i, &game);
            let yaw = l.camera.yaw;
            l.turn_with_stick(&control, &pad, look, magnet, 0.1);
            l.camera.yaw - yaw
        };
        // 0.6 of the stick to the right: Halo 2's 30 degrees a second.
        let free = turned(None);
        assert!((free.to_degrees() + 3.0).abs() < 1e-3, "{free}");
        let ahead = Vec3::new(10.0, 0.0, 0.0);
        let on = turned(Some(magnet_at(ahead, 0.0)));
        assert!((on / free - 0.4).abs() < 1e-4, "{on}");
        let half = turned(Some(magnet_at(ahead, 3f32.to_radians())));
        assert!((half / free - 0.7).abs() < 1e-4, "{half}");
        let edge = turned(Some(magnet_at(ahead, 6f32.to_radians())));
        assert!((edge / free - 1.0).abs() < 1e-4, "{edge}");
    }

    #[test]
    fn adhesion_follows_an_enemy_only_while_a_stick_moves() {
        let mut game = h2sim::testing::game();
        let i = game.add_player();
        let control = PlayerControl::default();
        let look = (camera::Controls::default(), 1.0);
        let dt = 1.0 / 60.0;
        // An enemy 10 units ahead strafing right at a unit a second, for
        // a second.
        let follow = |pad: PadState| {
            let mut l = LocalPlayer::new(i, &game);
            let (f, r, _) = l.camera.basis();
            let (eye, yaw) = (l.camera.position, l.camera.yaw);
            for k in 0..=60 {
                let at = eye + f * 10.0 + r * (k as f32 * dt);
                l.turn_with_stick(&control, &pad, look, Some(magnet_at(at, 0.0)), dt);
            }
            l.camera.yaw - yaw
        };
        let strafing = PadState {
            movement: Vec2::X,
            ..PadState::default()
        };
        let followed = follow(strafing);
        let across = 0.1f32.atan();
        assert!((followed + 0.7 * across).abs() < 1e-4, "{followed}");
        // Hands off the controller: no help.
        assert_eq!(follow(PadState::default()), 0.0);
    }

    #[test]
    fn hud_state_follows_the_hands_and_the_zoom() {
        let five = tags::WidgetState {
            yes_unit: 0x90,
            no_unit: 0x140,
            ..Default::default()
        };
        assert!(!five.shows(unit_flags(false, 0), 0));
        assert!(five.shows(unit_flags(false, 1), 0));
        assert!(!five.shows(unit_flags(false, 2), 0));
        // The SMG's lone right hand display hides dual wielding.
        let single = tags::WidgetState {
            yes_unit: 0x10,
            no_unit: 0x20,
            ..Default::default()
        };
        assert!(single.shows(unit_flags(false, 0), 0));
        assert!(!single.shows(unit_flags(true, 0), 0));
    }

    /// A reticle `size` pixels across, centred on the crosshair, shown
    /// with the `yes_unit` flags.
    fn reticle(name: &str, size: f32, yes_unit: u16) -> HudWidget {
        HudWidget {
            registration: [0.5, 0.5],
            size: [size, size],
            ..widget(name, yes_unit)
        }
    }

    #[test]
    fn the_lines_under_the_crosshair_clear_the_lowered_reticle() {
        // The battle rifle's reticle and the rocket launcher's, the
        // tallest of the multiplayer weapons' (lockout.map: 70 and 126
        // pixels of a 1280x960 screen in full screen, 34 and 62 split).
        let views = [
            (1280.0, 720.0, ScreenSplit::Full),
            (1920.0, 1080.0, ScreenSplit::Full),
            (2560.0, 1440.0, ScreenSplit::Full),
            (1280.0, 360.0, ScreenSplit::Half),
            (960.0, 540.0, ScreenSplit::Quarter),
        ];
        for (w, h, split) in views {
            let hb = HudBuilder::for_view(w, h, split).with_crosshair(0.165);
            let t = hb.text_scale();
            let cross = hb.anchor(Anchor::Crosshair)[1];
            let full = split == ScreenSplit::Full;
            let sizes = if full { [70.0, 126.0] } else { [34.0, 62.0] };
            for size in sizes {
                let widgets = [reticle("crosshair", size, 0)];
                let flags = (unit_flags(false, 0), 0);
                let bottom = reticle_bottom(&hb, &widgets, flags, hud_role);
                let reticle = hb.widget_rect(&widgets[0]);
                assert_eq!(bottom, Some(reticle[3]));
                let under = under_reticle(&hb, bottom);
                // RELOAD's top is below the reticle, and no nearer the
                // crosshair than it sat under one in the middle.
                assert!(under > reticle[3], "{w}x{h} {size}: {under}");
                assert!(under >= cross + UNDER_CROSSHAIR * t - 1e-3);
                assert!(under > h * 0.5 + UNDER_CROSSHAIR * t, "lowered too");
            }
        }
        // The battle rifle's keeps the gap it had at 720p.
        let hb = HudBuilder::new(1280.0, 720.0).with_crosshair(0.165);
        let widgets = [reticle("crosshair", 70.0, 0)];
        let bottom = reticle_bottom(&hb, &widgets, (unit_flags(false, 0), 0), hud_role);
        assert_eq!(under_reticle(&hb, bottom), 360.0 * 1.165 + 48.0);
        // Only the reticles shown count: not the zoom ticks reaching down
        // from it zoomed in, nor one the state flags hide.
        let tick = HudWidget {
            offset: [-12.0, 40.0],
            size: [26.0, 176.0],
            ..widget("bottom_crosshair", tags::UNIT_ZOOM_LEVEL_1)
        };
        let hidden = reticle("crosshair", 500.0, tags::UNIT_ZOOM_LEVEL_1);
        let widgets = [widgets[0].clone(), tick, hidden];
        let flags = (unit_flags(false, 0), 0);
        assert_eq!(reticle_bottom(&hb, &widgets, flags, hud_role), bottom);
        assert_eq!(under_reticle(&hb, None), 360.0 * 1.165 + 48.0);
    }

    #[test]
    fn a_driver_without_a_seat_gun_gets_no_aim_assist() {
        let world = h2sim::testing::floor();
        let mut game = h2sim::testing::game();
        // Guns drawn to enemies 6 degrees out to 21 units, as the battle
        // rifle is.
        for w in &mut game.weapons {
            w.magnetism_angle = 6f32.to_radians();
            w.magnetism_range = 21.0;
        }
        let (me, enemy) = (game.add_player(), game.add_player());
        game.players[enemy].body.position = Vec3::new(8.0, 0.0, 0.0);
        // A Warthog with no gun of its own: a driver and a passenger.
        let seat = |role, y: f32| SeatDef {
            role,
            position: Vec3::new(0.0, y, 0.4),
            entry: Vec3::new(0.0, y * 3.0, 0.4),
            entry_radius: 1.0,
            eye: Vec3::new(0.0, y, 0.6),
            exposed: true,
            third_person: true,
            weapon: None,
            alt_weapon: None,
            pivot: None,
            turret: None,
            pitch_range: [-0.8, 0.8],
            animation: String::new(),
            ai_only: false,
            camera: Vec::new(),
        };
        let jeep = h2sim::vehicle::VehicleDef {
            name: "warthog".into(),
            seats: vec![seat(SeatRole::Driver, 0.2), seat(SeatRole::Passenger, -0.2)],
            ..Default::default()
        };
        let spawn = h2sim::game::VehicleSpawn {
            def: 0,
            position: Vec3::new(0.0, -3.0, 0.0),
            yaw: 0.0,
            respawn: 30.0,
        };
        game.set_vehicles(vec![jeep], vec![spawn]);
        // Aimed right at the enemy, from wherever they sit.
        let aimed = |game: &Game| {
            let mut l = LocalPlayer::new(me, game);
            let q = &game.players[enemy].body;
            let at = q.position + Vec3::Z * q.height() * 0.5;
            l.camera = FlyCamera::looking_at(l.camera.position, at);
            l
        };
        let l = aimed(&game);
        assert!(l.gun(&game).is_some());
        assert_eq!(l.magnet(&game, &world).map(|m| m.player), Some(enemy));
        // Driving, the trigger is the horn: the battle rifle they carry
        // neither slows nor steers the wheel.
        game.enter_vehicle(me, 0, 0);
        assert_eq!(game.players[me].seat, Some((0, 0)));
        let l = aimed(&game);
        assert!(l.gun(&game).is_none());
        assert!(l.magnet(&game, &world).is_none());
        // A passenger aims their own gun.
        game.enter_vehicle(me, 0, 1);
        assert_eq!(game.players[me].seat, Some((0, 1)));
        let l = aimed(&game);
        assert_eq!(l.magnet(&game, &world).map(|m| m.player), Some(enemy));
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

    /// A controller with `buttons` held, under the button `layout`.
    fn pressing(layout: ButtonLayout, buttons: &[PadButton]) -> PadState {
        let mut held = ButtonSet::default();
        for &b in buttons {
            held.insert(b);
        }
        let reading = PadReading {
            held,
            ..PadReading::default()
        };
        reading.state(layout, StickLayout::Default)
    }

    /// Player `i` takes a second gun in their left hand.
    fn dual_wield(game: &mut Game, i: usize) {
        game.players[i].left = game.players[i].weapons.first().cloned();
        assert!(dual_wielding(game, i));
    }

    #[test]
    fn clicking_the_right_stick_dual_wielding_fires_nothing() {
        let mut game = h2sim::testing::game();
        let i = game.add_player();
        let l = LocalPlayer::new(i, &game);
        let pad = pressing(ButtonLayout::Default, &[PadButton::RightStick]);
        assert!(l.command(&game, None, Some(pad)).zoom);
        // Zoom fires the left gun, so the click doesn't count.
        dual_wield(&mut game, i);
        assert!(!l.command(&game, None, Some(pad)).zoom);
    }

    #[test]
    fn bumper_jumper_jumps_with_lb_and_swaps_grenades_with_a() {
        let mut game = h2sim::testing::game();
        let i = game.add_player();
        let l = LocalPlayer::new(i, &game);
        let press = |layout, b| l.command(&game, None, Some(pressing(layout, &[b])));
        let bj = |b| press(ButtonLayout::BumperJumper, b);
        let c = bj(PadButton::LB);
        assert!(c.jump && !c.vision);
        let c = bj(PadButton::A);
        assert!(c.switch_grenade && !c.jump);
        let c = bj(PadButton::RB);
        assert!(c.melee && !c.switch_grenade);
        let c = bj(PadButton::B);
        assert!(c.action && !c.melee);
        let c = bj(PadButton::X);
        assert!(c.vision && !c.reload && !c.action);
        assert!(bj(PadButton::Y).switch_weapon);
        // Halo 2's default, for comparison: LB the flashlight, A jump.
        assert!(press(ButtonLayout::Default, PadButton::LB).vision);
        assert!(press(ButtonLayout::Default, PadButton::A).jump);
        assert!(press(ButtonLayout::Default, PadButton::X).action);
    }

    #[test]
    fn boxers_left_trigger_melees_or_fires_the_left_gun() {
        let mut game = h2sim::testing::game();
        let i = game.add_player();
        let l = LocalPlayer::new(i, &game);
        let pad = pressing(ButtonLayout::Boxer, &[PadButton::LT]);
        let c = l.command(&game, None, Some(pad));
        assert!(c.melee && !c.throw_grenade);
        dual_wield(&mut game, i);
        let c = l.command(&game, None, Some(pad));
        assert!(c.throw_grenade && !c.melee);
    }

    #[test]
    fn boxers_b_throws_grenades_but_never_fires_the_left_gun() {
        let mut game = h2sim::testing::game();
        let i = game.add_player();
        let l = LocalPlayer::new(i, &game);
        let pad = pressing(ButtonLayout::Boxer, &[PadButton::B]);
        let c = l.command(&game, None, Some(pad));
        assert!(c.throw_grenade && !c.melee);
        // Dual wielding there are no grenades: B melees, as Halo 2's
        // Boxer does (its left trigger fires the left gun).
        dual_wield(&mut game, i);
        let c = l.command(&game, None, Some(pad));
        assert!(!c.throw_grenade && c.melee);
    }

    #[test]
    fn dual_wield_inversion_swaps_the_triggers_with_two_guns() {
        let mut game = h2sim::testing::game();
        let i = game.add_player();
        let mut l = LocalPlayer::new(i, &game);
        l.controls.dual_wield_inversion = true;
        let rt = pressing(ButtonLayout::Default, &[PadButton::RT]);
        let lt = pressing(ButtonLayout::Default, &[PadButton::LT]);
        // One gun: as ever.
        let c = l.command(&game, None, Some(rt));
        assert!(c.fire && !c.throw_grenade);
        assert!(l.command(&game, None, Some(lt)).throw_grenade);
        // Two: the right trigger fires the left gun, and the left the right.
        dual_wield(&mut game, i);
        let c = l.command(&game, None, Some(rt));
        assert!(c.throw_grenade && !c.fire);
        let c = l.command(&game, None, Some(lt));
        assert!(c.fire && !c.throw_grenade);
        l.controls.dual_wield_inversion = false;
        assert!(l.command(&game, None, Some(rt)).fire);
        // Boxer's left trigger too.
        l.controls.dual_wield_inversion = true;
        let boxer_lt = pressing(ButtonLayout::Boxer, &[PadButton::LT]);
        assert!(l.command(&game, None, Some(boxer_lt)).fire);
    }

    #[test]
    fn b_that_closes_the_pause_menu_does_not_melee() {
        let mut game = h2sim::testing::game();
        let i = game.add_player();
        let mut l = LocalPlayer::new(i, &game);
        l.pad = Some(pad(0));
        let mut pads = Pads::scripted();
        // B pressed on the pause menu: the menu spends it.
        pads.script("down 0 B");
        pads.events();
        pads.hold_off(pad(0), PadButton::B);
        // The game comes back with B still held: no melee.
        let c = l.command(&game, None, l.pad_state(&pads));
        assert!(!c.melee);
        // Let go and pressed again, it melees.
        pads.script("up 0 B");
        pads.events();
        assert!(!l.command(&game, None, l.pad_state(&pads)).melee);
        pads.script("down 0 B");
        pads.events();
        assert!(l.command(&game, None, l.pad_state(&pads)).melee);
        // A controller taking a player over: all it holds is spent.
        pads.script("down 0 RT");
        pads.events();
        pads.hold_off_all(pad(0));
        let c = l.command(&game, None, l.pad_state(&pads));
        assert!(!c.fire && !c.melee);
    }

    #[test]
    fn x_reloads_when_pressed_and_held_only_picks_up() {
        let mut game = h2sim::testing::game();
        let i = game.add_player();
        let mut l = LocalPlayer::new(i, &game);
        let x = pressing(ButtonLayout::Default, &[PadButton::X]);
        let c = l.command(&game, None, Some(x));
        assert!(c.action && !c.reload);
        l.taps.pad.insert(Function::Reload);
        let c = l.command(&game, None, Some(x));
        assert!(c.action && c.reload);
    }

    #[test]
    fn the_view_levels_out_walking_forward_with_look_centering() {
        let game = {
            let mut g = h2sim::testing::game();
            g.add_player();
            g
        };
        let control = PlayerControl::default();
        let forward = PadState {
            movement: Vec2::new(0.0, 1.0),
            ..PadState::default()
        };
        let walk = |l: &mut LocalPlayer, pad: &PadState, speed: f32, secs: f32| {
            for _ in 0..(secs * 60.0) as usize {
                l.center_look(&control, pad, Some(speed), false, 1.0 / 60.0);
            }
        };
        let mut l = LocalPlayer::new(0, &game);
        l.camera.pitch = -0.6;
        // Off by default (a guess: Halo 2's own default isn't known).
        walk(&mut l, &forward, 2.25, 2.0);
        assert_eq!(l.camera.pitch, -0.6);
        l.controls.look_centering = true;
        // Half a second (15 ticks) before it starts.
        walk(&mut l, &forward, 2.25, 0.4);
        assert_eq!(l.camera.pitch, -0.6);
        walk(&mut l, &forward, 2.25, 1.0);
        let after = l.camera.pitch;
        assert!(after > -0.6 && after < 0.0, "{after}");
        walk(&mut l, &forward, 2.25, 10.0);
        assert!(l.camera.pitch.abs() < 0.05 && l.camera.pitch <= 0.0, "{}", l.camera.pitch);
        // Not looking up or down, not strafing only, not riding, not with
        // an enemy in the aim assist, not standing still.
        let looking = PadState {
            look: Vec2::new(0.0, 0.3),
            ..forward
        };
        let strafing = PadState {
            movement: Vec2::new(1.0, 0.3),
            ..forward
        };
        for (pad, walking, assisted) in [
            (looking, Some(2.25), false),
            (strafing, Some(2.25), false),
            (forward, None, false),
            (forward, Some(2.25), true),
            (forward, Some(0.0), false),
        ] {
            l.camera.pitch = 0.5;
            l.level_time = 0.0;
            for _ in 0..120 {
                l.center_look(&control, &pad, walking, assisted, 1.0 / 60.0);
            }
            assert_eq!(l.camera.pitch, 0.5);
        }
    }

    #[test]
    fn green_thumb_melees_with_the_right_stick_and_zooms_with_b() {
        let mut game = h2sim::testing::game();
        let i = game.add_player();
        let l = LocalPlayer::new(i, &game);
        let stick = pressing(ButtonLayout::GreenThumb, &[PadButton::RightStick]);
        let c = l.command(&game, None, Some(stick));
        assert!(c.melee && !c.zoom);
        let b = pressing(ButtonLayout::GreenThumb, &[PadButton::B]);
        let c = l.command(&game, None, Some(b));
        assert!(c.zoom && !c.melee);
        // Dual wielding, B does nothing.
        dual_wield(&mut game, i);
        let c = l.command(&game, None, Some(b));
        assert!(!c.zoom && !c.melee && !c.throw_grenade);
        assert!(l.command(&game, None, Some(stick)).melee);
    }

    #[test]
    fn southpaw_fires_with_the_left_trigger() {
        let mut game = h2sim::testing::game();
        let i = game.add_player();
        let l = LocalPlayer::new(i, &game);
        let press = |b| l.command(&game, None, Some(pressing(ButtonLayout::Southpaw, &[b])));
        let c = press(PadButton::LT);
        assert!(c.fire && !c.throw_grenade);
        let c = press(PadButton::RT);
        assert!(c.throw_grenade && !c.fire);
    }

    #[test]
    fn legacy_turns_with_the_left_stick() {
        let mut game = h2sim::testing::game();
        let i = game.add_player();
        let control = PlayerControl::default();
        let look = (camera::Controls::default(), 1.0);
        let reading = PadReading {
            left: Vec2::new(0.6, 0.0),
            ..PadReading::default()
        };
        let turned = |sticks| {
            let pad = reading.state(ButtonLayout::Default, sticks);
            let mut l = LocalPlayer::new(i, &game);
            let yaw = l.camera.yaw;
            l.turn_with_stick(&control, &pad, look, None, 0.1);
            (l.camera.yaw - yaw, pad.movement)
        };
        // Legacy: the left stick turns, at the right stick's rate.
        let (turn, movement) = turned(StickLayout::Legacy);
        assert!((turn.to_degrees() + 3.0).abs() < 1e-3, "{turn}");
        assert_eq!(movement, Vec2::ZERO);
        // By default it strafes.
        let (turn, movement) = turned(StickLayout::Default);
        assert_eq!(turn, 0.0);
        assert_eq!(movement, Vec2::new(0.6, 0.0));
    }

    #[test]
    fn taps_resolve_when_the_tick_runs() {
        let mut game = h2sim::testing::game();
        let i = game.add_player();
        let mut l = LocalPlayer::new(i, &game);
        // Boxer's left trigger tapped and let go between ticks, with one
        // gun in hand...
        let lt = ButtonLayout::Boxer.function(PadButton::LT).unwrap();
        l.taps.pad.insert(lt);
        let c = l.command(&game, None, None);
        assert!(c.melee && !c.throw_grenade);
        // ...but a second one by the tick: it fires that.
        dual_wield(&mut game, i);
        let c = l.command(&game, None, None);
        assert!(c.throw_grenade && !c.melee);
    }

    #[test]
    fn pick_up_and_vehicle_prompts_name_the_layouts_button() {
        let mut game = h2sim::testing::game();
        let i = game.add_player();
        let mut l = LocalPlayer::new(i, &game);
        l.keyboard = true;
        assert_eq!(l.prompt_button("E", Function::Reload), "E");
        assert_eq!(l.prompt_button("Q", Function::SwitchWeapons), "Q");
        // Once a controller's A takes player one over, its buttons.
        l.pad = Some(pad(0));
        assert_eq!(l.prompt_button("E", Function::Reload), "X");
        assert_eq!(l.prompt_button("Q", Function::SwitchWeapons), "Y");
        l.controls.buttons = ButtonLayout::BumperJumper;
        assert_eq!(l.prompt_button("E", Function::Reload), "B");
        l.controls.buttons = ButtonLayout::Recon;
        assert_eq!(l.prompt_button("E", Function::Reload), "RB");
        // Back at the keyboard, keys again, till the controller's used.
        l.typing = true;
        assert_eq!(l.prompt_button("E", Function::Reload), "E");
        l.hint("PRESS {FLASHLIGHT} FOR ACTIVE CAMOUFLAGE");
        assert_eq!(l.messages.last().unwrap().0, "PRESS V FOR ACTIVE CAMOUFLAGE");
        l.typing = false;
        l.hint("PRESS {FLASHLIGHT} FOR ACTIVE CAMOUFLAGE");
        assert_eq!(l.messages.last().unwrap().0, "PRESS X FOR ACTIVE CAMOUFLAGE");
        assert_eq!(l.prompt_button("Q", Function::SwitchWeapons), "Y");
    }

    fn pad(n: usize) -> PadId {
        PadId(n)
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
        assert_eq!(new_pad_for(&locals, PadButton::Start), None);
        assert_eq!(new_pad_for(&locals, PadButton::A), Some(0));
        // A guest whose controller went is the one Start (or A) is for.
        locals[1].pad = None;
        locals[1].lost_pad = Some(pad(1));
        assert_eq!(new_pad_for(&locals, PadButton::Start), Some(1));
        assert_eq!(new_pad_for(&locals, PadButton::A), Some(1));
        locals[0].lost_pad = None;
        assert_eq!(new_pad_for(&locals, PadButton::A), Some(1));
        // No one lost theirs: A takes player one at the keyboard, unless
        // a controller already has them.
        locals[1].lost_pad = None;
        locals[1].pad = Some(pad(1));
        assert_eq!(new_pad_for(&locals, PadButton::A), Some(0));
        assert_eq!(new_pad_for(&locals, PadButton::Start), None);
        locals[0].pad = Some(pad(2));
        assert_eq!(new_pad_for(&locals, PadButton::A), None);
    }

    #[test]
    fn a_controller_back_waits_for_a_before_it_plays() {
        let mut game = h2sim::testing::game();
        let i = game.add_player();
        let mut l = LocalPlayer::new(i, &game);
        let mut pads = Pads::scripted();
        pads.script("down 0 RT");
        pads.events();
        l.pad = Some(pad(0));
        assert!(l.pad_state(&pads).is_some());
        // It went and came back: nothing from it until A (main.rs).
        l.lost_pad = Some(pad(0));
        assert!(l.pad_state(&pads).is_none());
        assert!(!l.command(&game, None, l.pad_state(&pads)).fire);
        l.lost_pad = None;
        assert!(l.command(&game, None, l.pad_state(&pads)).fire);
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
