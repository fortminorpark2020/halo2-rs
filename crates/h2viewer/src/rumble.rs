//! Controller vibration, as Halo 2 plays it. A damage effect (`jpt!`)
//! carries "player responses": for a player with shields up, without, or
//! either, how long each of the controller's two motors (the heavy low one
//! and the light high one) rumbles, and how hard over that time. Firing a
//! weapon plays its barrel's firing damage effect on the one firing;
//! being hit, meleed or caught in a blast plays that damage's on the one
//! hit. Each local player plays up to eight at once, a ninth taking the
//! oldest's place, and their motors run at the sum (Halo 2's
//! `rumble_player_evaluate`, re-implemented from the CC0 decompilation).

use crate::{App, Mode};
use blam_cache::weapon::{ResponseKind, Rumble, TagFunction, Transition, Vibration};
use h2sim::game::{Event, GrenadeKind};

/// How many rumbles a player plays at once (Halo 2's).
const PLAYING: usize = 8;

impl App {
    /// What a game event rumbles: the controllers of the players here it
    /// touches, at full strength (Halo 2 plays a firing weapon's, a melee's
    /// and a blast's that way; the others are taken to be the same).
    pub(crate) fn rumble_event(&mut self, e: &Event) {
        let (scene, game) = (&self.scene, &self.game);
        let weapon = |w: usize| scene.weapons.get(w).map(|w| &w.rumble);
        for l in &mut self.locals {
            let me = l.player;
            let Some(p) = game.players.get(me) else {
                continue;
            };
            let near = |at: glam::Vec3, radius: f32| p.body.position.distance(at) <= radius;
            let shielded = p.shield > 0.0;
            let mut play = |responses: &[Vibration]| {
                if let Some(v) = response(responses, shielded) {
                    l.rumble.play(v, 1.0);
                }
            };
            match *e {
                Event::Shot {
                    player,
                    weapon: w,
                    hit_player,
                    ..
                } => {
                    let Some(r) = weapon(w) else { continue };
                    if player == me {
                        play(&r.fire);
                    } else if hit_player == Some(me) {
                        play(&r.impact);
                    }
                }
                Event::Melee {
                    player,
                    hit: Some(victim),
                } => {
                    let held = game
                        .players
                        .get(player)
                        .and_then(|p| p.weapons.get(p.current))
                        .and_then(|h| weapon(h.weapon));
                    if let Some(r) = held {
                        if player == me {
                            play(&r.melee_response);
                        } else if victim == me {
                            play(&r.melee);
                        }
                    }
                }
                Event::Impact {
                    weapon: w,
                    position,
                    hit_player,
                    exploded,
                    ..
                } => {
                    let Some(r) = weapon(w) else { continue };
                    if hit_player == Some(me) {
                        play(&r.impact);
                    }
                    if exploded && near(position, r.blast_radius) {
                        play(&r.detonation);
                    }
                }
                Event::Exploded { kind, position } => {
                    let g = &scene.grenades[match kind {
                        GrenadeKind::Frag => 0,
                        GrenadeKind::Plasma => 1,
                    }];
                    if near(position, g.blast_radius) {
                        play(&g.rumble);
                    }
                }
                _ => {}
            }
        }
    }

    /// The controllers' motors this frame: each player's rumbles, if
    /// their Controller Vibration is on; still while the game is paused
    /// or their menu is up, and off in the menus.
    pub(crate) fn update_rumble(&mut self, dt: f32) {
        let playing = self.mode == Mode::Playing && self.loading.is_none();
        if !playing {
            for l in &mut self.locals {
                l.rumble.clear();
            }
            self.pads.stop_rumble();
            return;
        }
        let paused = self.paused();
        let menus: Vec<bool> = (0..self.locals.len()).map(|k| self.menu_for(k)).collect();
        for (l, menu) in self.locals.iter_mut().zip(menus) {
            let level = l.rumble.frame(if paused { 0.0 } else { dt });
            let Some(pad) = l.pad else {
                continue;
            };
            let [low, high] = match paused || menu || !l.controls.vibration {
                true => [0.0; 2],
                false => level,
            };
            self.pads.rumble(pad, low, high);
        }
    }
}

/// One rumble playing: its motors, how hard (1 is as the tag has it), and
/// how long it has played.
#[derive(Clone, Debug, PartialEq)]
struct Playing {
    motors: [Rumble; 2],
    scale: f32,
    time: f32,
}

/// One local player's rumbles.
#[derive(Clone, Debug, Default)]
pub struct Rumbler {
    playing: Vec<Playing>,
}

impl Rumbler {
    /// Play `v` at `scale`: in a free place, or the one that has played
    /// longest.
    pub fn play(&mut self, v: &Vibration, scale: f32) {
        let new = Playing {
            motors: [v.low.clone(), v.high.clone()],
            scale,
            time: 0.0,
        };
        if self.playing.len() < PLAYING {
            self.playing.push(new);
            return;
        }
        if let Some(oldest) = (self.playing.iter_mut()).max_by(|a, b| a.time.total_cmp(&b.time)) {
            *oldest = new;
        }
    }

    /// Time goes on: what's done is dropped.
    pub fn step(&mut self, dt: f32) {
        for p in &mut self.playing {
            p.time += dt;
        }
        self.playing
            .retain(|p| p.motors.iter().any(|m| m.duration > p.time));
    }

    /// The motors now, low and high, 0 to 1: each the sum of what plays.
    pub fn level(&self) -> [f32; 2] {
        let mut level = [0.0f32; 2];
        for p in &self.playing {
            for (l, m) in level.iter_mut().zip(&p.motors) {
                if m.duration > p.time {
                    let t = (p.time / m.duration).clamp(0.0, 1.0);
                    *l += m.function.at(t) * p.scale;
                }
            }
        }
        level.map(|l| l.clamp(0.0, 1.0))
    }

    /// The motors for the frame starting now, then `dt` on: what began
    /// since the last frame reaches the motors for a frame at least, even
    /// a firing rumble shorter than a frame (the battle rifle's 0.034 s
    /// against a slow frame).
    pub fn frame(&mut self, dt: f32) -> [f32; 2] {
        let level = self.level();
        self.step(dt);
        level
    }

    pub fn clear(&mut self) {
        self.playing.clear();
    }
}

/// The response a damage effect plays on a player with shields up or not:
/// the first for them (one for either fits both), as Halo 2 picks it.
pub fn response(responses: &[Vibration], shielded: bool) -> Option<&Vibration> {
    let unfit = if shielded {
        ResponseKind::Unshielded
    } else {
        ResponseKind::Shielded
    };
    responses.iter().find(|r| r.kind != unfit)
}

/// A firing rumble for a weapon whose firing damage effect has none in the
/// PC maps (the SMG's, shotgun's and rocket launcher's are empty there):
/// the battle rifle's, or a launcher's like the tank cannon's. Estimates.
pub fn firing_fallback(heavy: bool) -> Vibration {
    let motor = |duration: f32, shape, from, to| Rumble {
        duration,
        function: TagFunction::Transition { shape, from, to },
    };
    let (low, high) = if heavy {
        (
            motor(0.1, Transition::Early, 1.0, 0.7),
            motor(0.1, Transition::Early, 1.0, 0.8),
        )
    } else {
        (
            motor(0.034, Transition::VeryEarly, 0.8, 0.3),
            motor(0.034, Transition::VeryEarly, 0.9, 0.05),
        )
    };
    Vibration {
        kind: ResponseKind::All,
        low,
        high,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The battle rifle's trigger, as lockout.map has it.
    fn br_trigger() -> Vibration {
        firing_fallback(false)
    }

    fn shielded_and_not() -> Vec<Vibration> {
        let mut shielded = br_trigger();
        shielded.kind = ResponseKind::Shielded;
        shielded.low.duration = 0.5;
        let mut unshielded = br_trigger();
        unshielded.kind = ResponseKind::Unshielded;
        unshielded.low.duration = 0.25;
        vec![shielded, unshielded]
    }

    #[test]
    fn a_rumble_shorter_than_a_frame_still_reaches_the_motors() {
        let mut r = Rumbler::default();
        r.play(&br_trigger(), 1.0);
        // A slow frame (a tenth of a second) after the shot.
        let [low, high] = r.frame(0.1);
        assert!((low - 0.8).abs() < 1e-5 && (high - 0.9).abs() < 1e-5);
        assert_eq!(r.frame(0.1), [0.0, 0.0]);
        // Paused: no time goes on, the rumble stays where it was.
        r.play(&br_trigger(), 1.0);
        r.frame(0.0);
        assert_eq!(r.playing.len(), 1);
    }

    #[test]
    fn a_rumble_follows_its_motors_curves_and_ends() {
        let mut r = Rumbler::default();
        assert_eq!(r.level(), [0.0, 0.0]);
        r.play(&br_trigger(), 1.0);
        let [low, high] = r.level();
        assert!((low - 0.8).abs() < 1e-5 && (high - 0.9).abs() < 1e-5);
        // Half way through, on its very early curve, down toward the end.
        r.step(0.017);
        let [low, high] = r.level();
        assert!(low < 0.8 && low > 0.3, "{low}");
        assert!(high < 0.9 && high > 0.05, "{high}");
        r.step(0.02);
        assert_eq!(r.level(), [0.0, 0.0]);
        assert!(r.playing.is_empty());
    }

    #[test]
    fn rumbles_add_up_to_full_strength_and_scale() {
        let mut r = Rumbler::default();
        r.play(&br_trigger(), 0.5);
        let [low, _] = r.level();
        assert!((low - 0.4).abs() < 1e-5, "{low}");
        r.play(&br_trigger(), 1.0);
        r.play(&br_trigger(), 1.0);
        assert_eq!(r.level(), [1.0, 1.0]);
    }

    #[test]
    fn a_ninth_rumble_takes_the_oldests_place() {
        let mut r = Rumbler::default();
        let mut long = br_trigger();
        long.low.duration = 10.0;
        r.play(&long, 0.1);
        for k in 0..PLAYING - 1 {
            r.step(0.001);
            r.play(&long, 0.01 * k as f32);
        }
        assert_eq!(r.playing.len(), PLAYING);
        r.step(0.001);
        r.play(&long, 0.0);
        assert_eq!(r.playing.len(), PLAYING);
        // The first (0.1) went.
        assert!(r.playing.iter().all(|p| p.scale < 0.1));
        r.clear();
        assert_eq!(r.level(), [0.0, 0.0]);
    }

    #[test]
    fn shields_pick_the_response() {
        let both = shielded_and_not();
        assert_eq!(response(&both, true).unwrap().low.duration, 0.5);
        assert_eq!(response(&both, false).unwrap().low.duration, 0.25);
        // One for either fits both.
        let all = [br_trigger()];
        assert!(response(&all, true).is_some() && response(&all, false).is_some());
        assert!(response(&both[..1], false).is_none());
        assert!(response(&[], true).is_none());
    }

    #[test]
    fn transition_shapes_go_from_one_value_to_the_other() {
        for shape in [
            Transition::Linear,
            Transition::Early,
            Transition::VeryEarly,
            Transition::Late,
            Transition::VeryLate,
            Transition::Cosine,
        ] {
            let f = TagFunction::Transition {
                shape,
                from: 0.8,
                to: 0.2,
            };
            assert!((f.at(0.0) - 0.8).abs() < 1e-5, "{shape:?}");
            assert!((f.at(1.0) - 0.2).abs() < 1e-5, "{shape:?}");
            let mid = f.at(0.5);
            assert!(mid < 0.8 && mid > 0.2, "{shape:?} {mid}");
        }
        // Early gets there sooner than late.
        let at = |shape| {
            TagFunction::Transition {
                shape,
                from: 1.0,
                to: 0.0,
            }
            .at(0.5)
        };
        assert!(at(Transition::VeryEarly) < at(Transition::Early));
        assert!(at(Transition::Early) < at(Transition::Linear));
        assert!(at(Transition::Linear) < at(Transition::Late));
        assert!(at(Transition::Late) < at(Transition::VeryLate));
        assert_eq!(TagFunction::Constant(0.3).at(0.7), 0.3);
        assert_eq!(TagFunction::None.at(0.7), 1.0);
    }

    #[test]
    fn tag_functions_read_as_halo_2_stores_them() {
        // lockout.map's battle_rifle_trigger, its low motor: a transition,
        // very early, 0.8 to 0.3.
        let mut data = vec![0u8; 0x1C];
        data[0] = 2;
        data[2] = 2;
        data[0x14..0x18].copy_from_slice(&0.8f32.to_le_bytes());
        data[0x18..0x1C].copy_from_slice(&0.3f32.to_le_bytes());
        assert_eq!(
            TagFunction::parse(&data),
            TagFunction::Transition {
                shape: Transition::VeryEarly,
                from: 0.8,
                to: 0.3
            }
        );
        let mut constant = vec![0u8; 0x18];
        constant[0] = 1;
        constant[0x14..0x18].copy_from_slice(&0.25f32.to_le_bytes());
        assert_eq!(TagFunction::parse(&constant), TagFunction::Constant(0.25));
        assert_eq!(TagFunction::parse(&[]), TagFunction::None);
        assert!(matches!(TagFunction::parse(&[9, 0, 0]), TagFunction::Other(_)));
    }
}
