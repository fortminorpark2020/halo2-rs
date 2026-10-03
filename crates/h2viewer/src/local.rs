//! A player at this computer: their camera, first person weapon, HUD and
//! controls. Splitscreen is several of these, each drawn in its own part of
//! the window.

use crate::camera::{self, FlyCamera};
use crate::effects;
use crate::gpu::{self, hud_mode, DrawCall, SpriteVertex};
use crate::hud::{self, HudBuilder};
use crate::input::PadState;
use crate::rig;
use crate::scene::{Scene, Vertex, WeaponAssets};
use gilrs::GamepadId;
use glam::{Mat4, Vec3};
use h2sim::game::{GrenadeKind, Spartan, TICK};
use h2sim::{Command, Game, WeaponState, World};
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
pub fn armor_colors(player: usize) -> [[f32; 3]; 2] {
    let primary = ARMOR_COLORS[player % ARMOR_COLORS.len()];
    let secondary = ARMOR_COLORS[(player + 8) % ARMOR_COLORS.len()];
    [primary, secondary]
}

/// "battle_rifle" -> "BATTLE RIFLE".
pub fn display_name(name: &str) -> String {
    name.replace('_', " ").to_uppercase()
}

pub fn player_name(me: usize, i: usize) -> String {
    if i == me {
        "YOU".into()
    } else {
        format!("PLAYER {}", i + 1)
    }
}

/// A kill feed line, as Halo 2 words it.
pub fn kill_message(me: usize, killer: Option<usize>, victim: usize) -> String {
    let v = player_name(me, victim);
    match killer {
        Some(k) if k == victim && k == me => "YOU KILLED YOURSELF".into(),
        Some(k) if k == victim => format!("{v} COMMITTED SUICIDE"),
        Some(k) => format!("{} KILLED {v}", player_name(me, k)),
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
}

/// What a player's first person view should react to this frame.
#[derive(Default)]
pub struct ViewEvents {
    pub fired: bool,
    pub melee: bool,
    pub thrown: bool,
    pub reload: Option<bool>,
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

pub struct LocalPlayer {
    /// The game player this person controls.
    pub player: usize,
    /// Played with the keyboard and mouse (player one).
    pub keyboard: bool,
    pub pad: Option<GamepadId>,
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
    /// Kill feed and pickups, newest last, with seconds left on screen.
    pub messages: Vec<(String, f32)>,
    /// A standing line at the top of the view (LAN games to join).
    pub notice: Option<String>,
    pub view: ViewEvents,
}

impl LocalPlayer {
    pub fn new(player: usize, game: &Game) -> LocalPlayer {
        let p = &game.players[player];
        let eye = p.eye();
        LocalPlayer {
            player,
            keyboard: false,
            pad: None,
            flying: false,
            camera: FlyCamera::looking_at(eye, eye + glam::vec3(p.yaw.cos(), p.yaw.sin(), 0.0)),
            taps: Taps::default(),
            eyes: (eye, eye),
            bob_phase: 0.0,
            animator: rig::Animator::default(),
            shown_weapon: None,
            shots_fired: 0,
            messages: Vec::new(),
            notice: None,
            view: ViewEvents::default(),
        }
    }

    pub fn me<'a>(&self, game: &'a Game) -> &'a Spartan {
        &game.players[self.player]
    }

    /// Seen in first person: alive and walking.
    pub fn first_person(&self, game: &Game) -> bool {
        !self.flying && self.me(game).alive
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
        self.current(scene, game)
            .map(|(w, s)| w.def.magnification(s.zoom))
            .unwrap_or(1.0)
    }

    pub fn message(&mut self, text: String) {
        self.messages.push((text, 5.0));
        if self.messages.len() > 4 {
            self.messages.remove(0);
        }
    }

    /// This tick's controls, from the keyboard and mouse and/or a controller.
    pub fn command(&self, keyboard: Option<&Keyboard>, pad: Option<PadState>) -> Command {
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
        }
        if let Some(p) = pad {
            if p.left != glam::Vec2::ZERO {
                cmd.movement = p.left;
            }
            cmd.jump |= p.jump;
            cmd.crouch |= p.crouch;
            cmd.fire |= p.fire;
            cmd.zoom |= p.zoom;
            cmd.action |= p.action;
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
        if me.alive {
            let a = (pending / TICK).clamp(0.0, 1.0);
            self.camera.position = self.eyes.0.lerp(self.eyes.1, a);
            if me.body.grounded {
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
    }

    /// Pick and advance the first person animation: bring a new weapon up,
    /// fire, melee, throw and reload on cue, otherwise idle.
    pub fn animate_view_model(&mut self, scene: &Scene, game: &Game, dt: f32) {
        let view = std::mem::take(&mut self.view);
        let weapon = self.me(game).held().map(|h| h.weapon);
        let Some(rig) = weapon
            .and_then(|w| scene.weapons.get(w))
            .and_then(|w| w.rig.as_ref())
        else {
            self.shown_weapon = weapon;
            return;
        };
        if view.switched || weapon != self.shown_weapon {
            self.shown_weapon = weapon;
            self.animator.play(rig.find("first_person:ready", 0), false);
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
        if !self.me(game).alive {
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
        let (model, muzzle_frame) = match (&weapon.rig, &scene.arms) {
            (Some(rig), Some(arms)) if !self.animator.pose().is_empty() => {
                // Arms and gun posed by the animation, in camera space.
                let world = rig.world(self.animator.pose());
                let frame = self.view_frame(game);
                out.posed.push((
                    arms.mesh,
                    arms.skin.pose(&rig.arms_skin(&world, &arms.skeleton)),
                ));
                out.posed.push((
                    mesh,
                    weapon.skin.pose(&rig.gun_skin(&world, &weapon.skeleton)),
                ));
                out.view_models.push(DrawCall {
                    mesh: arms.mesh,
                    model: frame,
                    light,
                    colors: Some(armor_colors(self.player)),
                });
                let muzzle = rig
                    .gun_node_world(&world, weapon.muzzle_node)
                    .map_or(frame, |m| frame * m);
                (frame, muzzle)
            }
            _ => {
                let m = self.view_model_matrix(game, weapon, state);
                let node = weapon
                    .skeleton
                    .inverse_bind
                    .get(weapon.muzzle_node)
                    .map_or(Mat4::IDENTITY, Mat4::inverse);
                (m, m * node)
            }
        };
        out.view_models.push(DrawCall {
            mesh,
            model,
            light,
            colors: None,
        });
        if state.since_shot < 0.05 {
            let muzzle = muzzle_frame.transform_point3(Vec3::from(weapon.muzzle));
            let size = 0.012 + 0.006 * (state.since_shot * 300.0).sin().abs();
            effects::quad(
                &mut out.view_sprites,
                muzzle,
                r * size,
                u * size,
                [1.0, 0.8, 0.4, 0.9],
            );
        }
        out
    }

    /// Projection for the first person weapon, which ignores zoom.
    pub fn view_model_proj(&self, aspect: f32) -> Mat4 {
        camera::projection(aspect, 1.0, 0.005, 10.0) * self.camera.view()
    }

    pub fn build_hud(&self, scene: &Scene, game: &Game, w: f32, h: f32) -> Vec<gpu::HudBatch> {
        let mut hb = HudBuilder::new(w, h);
        let font = scene.hud_font;
        let me = self.me(game);
        let s = hb.scale();
        let shield = me.shield / game.rules.shield.max(1.0);
        // The shield meter flashes red while the shields are down.
        let flash = shield < 0.25 && me.alive && (game.time * 4.0).fract() < 0.5;
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
        if let Some(notice) = &self.notice {
            hb.text(font, [w * 0.5, 64.0 * s], 8.0 * s, notice, hud::BLUE);
        }
        // Your score, and the best of everyone else's under it.
        let best_other = (0..game.players.len())
            .filter(|&i| i != self.player)
            .map(|i| game.players[i].kills)
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
        if let Some(winner) = game.winner {
            let text = if winner == self.player {
                "YOU WIN".to_string()
            } else {
                format!("{} WINS", player_name(self.player, winner))
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
        if let Some(weapon) = game.swap_prompt(self.player) {
            if let Some(a) = scene.weapons.get(weapon) {
                let button = if self.keyboard { "E" } else { "X" };
                let text = format!("HOLD {button} TO PICK UP {}", display_name(&a.def.name));
                hb.text(
                    font,
                    [w * 0.5, h * 0.5 + 64.0 * s],
                    9.0 * s,
                    &text,
                    hud::BLUE,
                );
            }
        }
        let Some((weapon, state)) = self.current(scene, game) else {
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
}

/// Where each of `n` splitscreen views goes in a `w` x `h` window: one fills
/// it, two split it top and bottom, three or four take a quarter each.
pub fn viewports(n: usize, w: u32, h: u32) -> Vec<[u32; 4]> {
    let (hw, hh) = (w / 2, h / 2);
    match n {
        0 | 1 => vec![[0, 0, w, h]],
        2 => vec![[0, 0, w, hh], [0, hh, w, h - hh]],
        _ => [
            [0, 0, hw, hh],
            [hw, 0, w - hw, hh],
            [0, hh, hw, h - hh],
            [hw, hh, w - hw, h - hh],
        ][..n.min(4)]
            .to_vec(),
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
    fn kill_feed_wording() {
        assert_eq!(kill_message(0, Some(0), 1), "YOU KILLED PLAYER 2");
        assert_eq!(kill_message(0, Some(1), 0), "PLAYER 2 KILLED YOU");
        assert_eq!(kill_message(0, Some(0), 0), "YOU KILLED YOURSELF");
        assert_eq!(kill_message(0, None, 2), "PLAYER 3 DIED");
    }

    #[test]
    fn splitscreen_views_cover_the_window() {
        for n in 1..=4 {
            let area: u32 = viewports(n, 1280, 721).iter().map(|v| v[2] * v[3]).sum();
            let expected = if n == 3 {
                1280 * 721 - 640 * 361
            } else {
                1280 * 721
            };
            assert_eq!(area, expected, "{n} views");
        }
        assert_eq!(viewports(2, 1280, 720)[1], [0, 360, 1280, 360]);
    }
}
