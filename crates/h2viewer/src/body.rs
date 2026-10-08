//! Third person Spartans: Master Chief's model posed by his animation graph
//! (idle, running, jumping, crouching, reloading, melee...) and holding the
//! weapon in hand.
//!
//! Halo 2 builds a biped's pose in layers: a base animation for the stance and
//! movement (`combat:rifle:move_front`), a replacement for how the hands grip
//! the weapon (`combat:rifle:br:grip`) and replacements for actions
//! (`combat:rifle:br:reload_1`). Animation names go from specific to general,
//! so a missing `crouch:pistol:hp:reload_1` falls back to
//! `combat:pistol:hp:reload_1`, `combat:pistol:reload_1` and so on.

use crate::rig::{tag_quat, world_matrices, NodePose, Skeleton, SkinnedMesh};
use blam_cache::animation::{Animation, AnimationGraph, AnimationKind, FrameEvent, FRAME_RATE};
use blam_cache::model::Marker;
use blam_cache::physics::BipedPhysics;
use glam::{Mat4, Quat, Vec2, Vec3};
use std::collections::HashMap;

/// Seconds to fade between base animations.
const BASE_BLEND: f32 = 0.2;
/// Seconds to fade actions in and out.
const ACTION_FADE: f32 = 0.1;
/// Run speed at which movement animations that don't say how fast they go
/// play at their own pace (the globals' forward run speed).
const RUN_SPEED: f32 = 2.25;
/// Slower than this (world units a second) a body stands still.
const STILL: f32 = 0.15;
/// Seconds off the ground before a body shows it's in the air, so a step
/// down a stair or over a bump doesn't flick it into the jumping pose.
/// (Our choice: about three of Halo's 30 a second ticks.)
const AIRBORNE_AFTER: f32 = 0.1;
/// The slowest and fastest a walking or running animation is played, as
/// a share of its own pace (a limit for odd data: with the tags' own
/// paces it stays within about 0.3 to 2.5).
const MOVE_RATES: (f32, f32) = (0.25, 2.5);

/// The model, its animations and where the weapon goes.
pub struct BodyRig {
    pub graph: AnimationGraph,
    pub skeleton: Skeleton,
    pub skin: SkinnedMesh,
    /// Default pose of each graph node.
    defaults: Vec<NodePose>,
    /// For each model node, the graph node driving it.
    nodes: Vec<Option<usize>>,
    /// Animation index by full name and by name without its `:varN` suffix.
    names: HashMap<String, usize>,
    /// Where the weapons go in each hand.
    hands: [Option<Marker>; 2],
    /// When landing from a fall shows.
    landing: Landing,
}

/// When a body shows landing from a fall: values from its biped tag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Landing {
    /// Coming down at least this fast (world units a second) is a soft
    /// landing, at least `hard_speed` a hard one.
    pub soft_speed: f32,
    pub hard_speed: f32,
    /// The longest each lasts, in seconds (0: as long as its animation).
    pub soft_time: f32,
    pub hard_time: f32,
}

impl Landing {
    pub fn of(biped: &BipedPhysics) -> Landing {
        Landing {
            soft_speed: biped.soft_landing_speed,
            hard_speed: biped.hard_landing_speed,
            soft_time: biped.soft_landing_time,
            hard_time: biped.hard_landing_time,
        }
    }
}

/// How a weapon is held: its animation class and short code.
pub fn weapon_style(name: &str) -> (&'static str, &'static str) {
    match name {
        "battle_rifle" => ("rifle", "br"),
        "smg" => ("rifle", "smg"),
        "magnum" => ("pistol", "hp"),
        "shotgun" => ("rifle", "sg"),
        "sniper_rifle" => ("rifle", "sr"),
        "rocket_launcher" => ("missile", "rl"),
        "covenant_carbine" => ("rifle", "cb"),
        "beam_rifle" => ("rifle", "csr"),
        "plasma_rifle" | "brute_plasma_rifle" => ("rifle", "pr"),
        "plasma_pistol" => ("pistol", "pp"),
        "needler" => ("pistol", "ne"),
        "brute_shot" => ("support", "bs"),
        "flak_cannon" => ("missile", "fc"),
        "energy_blade" => ("sword", ""),
        "flag" => ("flag", ""),
        "ball" => ("ball", ""),
        "assault_bomb" => ("ball", "bomb"),
        _ => ("rifle", "any"),
    }
}

impl BodyRig {
    pub fn new(
        graph: AnimationGraph,
        skeleton: Skeleton,
        skin: SkinnedMesh,
        hands: [Option<Marker>; 2],
        landing: Landing,
    ) -> BodyRig {
        let defaults = graph
            .nodes
            .iter()
            .map(|n| {
                skeleton
                    .node(&n.name)
                    .map_or(NodePose::IDENTITY, |i| skeleton.default[i])
            })
            .collect();
        let nodes = skeleton.names.iter().map(|n| graph.node(n)).collect();
        let mut names = HashMap::new();
        for (i, a) in graph.animations.iter().enumerate() {
            if !a.decoded {
                continue;
            }
            names.entry(a.name.clone()).or_insert(i);
            if let Some((base, var)) = a.name.rsplit_once(':') {
                if var.starts_with("var") {
                    names.entry(base.to_string()).or_insert(i);
                }
            }
        }
        BodyRig {
            graph,
            skeleton,
            skin,
            defaults,
            nodes,
            names,
            hands,
            landing,
        }
    }

    /// The most specific animation for `what` in a stance holding a weapon.
    pub fn find(&self, stance: &str, (class, code): (&str, &str), what: &str) -> Option<usize> {
        let mut candidates = Vec::with_capacity(8);
        for s in [stance, "combat"] {
            if !code.is_empty() {
                candidates.push(format!("{s}:{class}:{code}:{what}"));
            }
            candidates.push(format!("{s}:{class}:{what}"));
            candidates.push(format!("{s}:rifle:{what}"));
            // Seats have some animations whatever the rider holds
            // ("ghost_d:grip").
            if s != "combat" && s != "crouch" {
                candidates.push(format!("{s}:{what}"));
            }
        }
        candidates
            .iter()
            .find_map(|n| self.names.get(n.as_str()).copied())
    }

    pub fn by_name(&self, name: &str) -> Option<usize> {
        self.names.get(name).copied()
    }

    pub fn duration(&self, anim: usize) -> f32 {
        self.graph
            .animations
            .get(anim)
            .map_or(0.0, |a| a.duration())
    }

    /// How fast and which way (x forward, y left) an animation carries the
    /// body at its own pace; zero when its tag doesn't say.
    fn speed(&self, anim: usize) -> Vec2 {
        self.graph
            .animations
            .get(anim)
            .map_or(Vec2::ZERO, |a| Vec2::from(a.speed()))
    }

    /// Where in `to` the legs are as they are `time` seconds into `from`:
    /// as far round its stride from the left foot coming down. (Turning
    /// from running forward to running sideways keeps the stride going.)
    fn same_stride(&self, from: usize, time: f32, to: usize) -> f32 {
        let (Some(a), Some(b)) = (
            self.graph.animations.get(from),
            self.graph.animations.get(to),
        ) else {
            return 0.0;
        };
        let stride = |x: &Animation| x.frame_count.saturating_sub(1).max(1) as f32 / FRAME_RATE;
        let foot = |x: &Animation| {
            x.event_frame(FrameEvent::LeftFoot)
                .map_or(0.0, |f| f as f32 / FRAME_RATE)
        };
        let share = ((time - foot(a)) / stride(a)).rem_euclid(1.0);
        foot(b) + share * stride(b)
    }

    /// Lay `anim` at `time` seconds over `pose`, `weight` of the way. A base
    /// animation sets every node; others only the nodes they animate.
    fn apply(&self, pose: &mut [NodePose], anim: usize, time: f32, looping: bool, weight: f32) {
        let Some(a) = self.graph.animations.get(anim) else {
            return;
        };
        let last = a.frame_count.saturating_sub(1).max(1) as f32;
        let frame = if looping {
            (time * FRAME_RATE) % last
        } else {
            (time * FRAME_RATE).min(last)
        };
        let base = a.kind == AnimationKind::Base;
        for (n, p) in pose.iter_mut().enumerate() {
            let rot = a.rotations.get(n).and_then(Option::as_ref);
            let trans = a.translations.get(n).and_then(Option::as_ref);
            if !base && rot.is_none() && trans.is_none() {
                continue;
            }
            let d = if base { self.defaults[n] } else { *p };
            let target = NodePose {
                rotation: rot.map_or(d.rotation, |t| tag_quat(t.sample(frame))),
                translation: trans.map_or(d.translation, |t| Vec3::from(t.sample(frame))),
                scale: d.scale,
            };
            *p = p.lerp(&target, weight);
        }
    }

    /// Bend the spine and head to look `pitch` radians up (or down), so
    /// the gun in the hands points where the Spartan aims.
    fn look(&self, pose: &mut [NodePose], pitch: f32) {
        for (name, share) in [("spine", 0.3), ("spine1", 0.45), ("head", 0.25)] {
            let Some(n) = self.graph.node(name) else {
                continue;
            };
            let parent = match usize::try_from(self.graph.nodes[n].parent) {
                Ok(p) => self.world(pose)[p].to_scale_rotation_translation().1,
                Err(_) => Quat::IDENTITY,
            };
            // Turned about the body's left-right axis, in the parent's space.
            let turn = Quat::from_rotation_y(-pitch * share);
            pose[n].rotation = parent.inverse() * turn * parent * pose[n].rotation;
        }
    }

    /// Graph node matrices relative to the object (origin at the feet,
    /// facing +x).
    pub fn world(&self, pose: &[NodePose]) -> Vec<Mat4> {
        let parents: Vec<i16> = self.graph.nodes.iter().map(|n| n.parent).collect();
        world_matrices(&parents, pose)
    }

    /// Skinning matrices for the model.
    pub fn skin_matrices(&self, world: &[Mat4]) -> Vec<Mat4> {
        (0..self.skeleton.names.len())
            .map(|i| {
                let w = self.nodes[i].and_then(|g| world.get(g)).copied();
                w.map_or(Mat4::IDENTITY, |w| w * self.skeleton.inverse_bind[i])
            })
            .collect()
    }

    /// Where the weapon's origin goes, relative to the object: in the
    /// right hand, or (dual wielding) the left.
    pub fn weapon_frame(&self, world: &[Mat4], left: bool) -> Mat4 {
        let Some(m) = self.hands[left as usize] else {
            return Mat4::IDENTITY;
        };
        let node = self.nodes.get(m.node as usize).copied().flatten();
        let hand = node.and_then(|g| world.get(g)).copied();
        hand.unwrap_or(Mat4::IDENTITY)
            * Mat4::from_rotation_translation(tag_quat(m.rotation), Vec3::from(m.translation))
    }
}

/// What a Spartan is doing, for picking animations.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyInput {
    /// Velocity in the body's frame: x forward, y left.
    pub velocity: Vec2,
    /// How fast it goes up (down when negative), world units a second.
    pub climb: f32,
    pub grounded: bool,
    pub crouching: bool,
    pub alive: bool,
    /// The weapon's animation class and code.
    pub style: (&'static str, &'static str),
    /// Riding: the seat's animations ("warthog_d").
    pub seat: Option<&'static str>,
    /// How far up (radians) they look; the upper body bends to aim there.
    pub pitch: f32,
}

/// One Spartan's animation state.
#[derive(Default)]
pub struct BodyAnimator {
    base: Option<usize>,
    base_time: f32,
    /// The base is a walking or running stride.
    striding: bool,
    previous: Option<(usize, f32)>,
    blend: f32,
    action: Option<usize>,
    action_time: f32,
    /// Seconds off the ground, and the fastest it has come down meanwhile.
    airborne_for: f32,
    falling: f32,
    /// Landing from a fall: the animation, and seconds of it left.
    landing: Option<(usize, f32)>,
}

impl BodyAnimator {
    /// Play a one-off action (`reload_1`, `melee_strike_1`, `throw_grenade`,
    /// `ready`...) over the movement.
    pub fn act(&mut self, rig: &BodyRig, input: &BodyInput, what: &str) {
        if let Some(a) = rig.find(stance(input), input.style, what) {
            self.action = Some(a);
            self.action_time = 0.0;
        }
    }

    /// Play an animation by its full name (a script's gesture) over the
    /// movement; false if the body has none called that.
    pub fn play(&mut self, rig: &BodyRig, name: &str) -> bool {
        let Some(a) = rig.by_name(name) else {
            return false;
        };
        self.action = Some(a);
        self.action_time = 0.0;
        true
    }

    /// Advance by `dt` and pose the body.
    pub fn update(&mut self, rig: &BodyRig, input: &BodyInput, dt: f32) -> Vec<NodePose> {
        let (want, rate, striding) = if !input.alive {
            // A fall it died in doesn't land it when it's back.
            self.landing = None;
            self.airborne_for = 0.0;
            self.falling = 0.0;
            (rig.by_name("combat:landing_dead"), 1.0, false)
        } else {
            self.watch_landing(rig, input, dt);
            self.movement(rig, input)
        };
        if want != self.base {
            let time = match (self.base, want) {
                (Some(from), Some(to)) if self.striding && striding => {
                    rig.same_stride(from, self.base_time, to)
                }
                _ => 0.0,
            };
            self.previous = self.base.map(|b| (b, self.base_time));
            self.base = want;
            self.base_time = time;
            self.blend = 0.0;
            if !input.alive {
                self.action = None;
            }
        }
        self.striding = striding;
        self.base_time += dt * rate;
        self.blend += dt;
        if let Some((_, t)) = &mut self.previous {
            *t += dt * rate;
        }

        let mut pose = rig.defaults.clone();
        let looping = input.alive;
        if let Some(b) = self.base {
            rig.apply(&mut pose, b, self.base_time, looping, 1.0);
        }
        let fade = (self.blend / BASE_BLEND).min(1.0);
        if let (Some((p, t)), true) = (self.previous, fade < 1.0) {
            let mut from = rig.defaults.clone();
            rig.apply(&mut from, p, t, true, 1.0);
            for (a, b) in pose.iter_mut().zip(&from) {
                *a = b.lerp(a, fade);
            }
        }
        if !input.alive {
            return pose;
        }
        if let Some(grip) = rig.find(stance(input), input.style, "grip") {
            rig.apply(&mut pose, grip, 0.0, false, 1.0);
        }
        if let Some(a) = self.action {
            self.action_time += dt;
            let length = rig.duration(a);
            if self.action_time >= length {
                self.action = None;
            } else {
                let w = (self.action_time / ACTION_FADE)
                    .min((length - self.action_time) / ACTION_FADE)
                    .clamp(0.0, 1.0);
                rig.apply(&mut pose, a, self.action_time, false, w);
            }
        }
        if input.pitch.abs() > 1e-3 {
            rig.look(&mut pose, input.pitch.clamp(-1.2, 1.2));
        }
        pose
    }
}

impl BodyAnimator {
    /// Keep track of time in the air, and on coming down fast enough
    /// start a soft or hard landing (which moving off cuts short).
    fn watch_landing(&mut self, rig: &BodyRig, input: &BodyInput, dt: f32) {
        if let Some((_, left)) = &mut self.landing {
            *left -= dt;
        }
        if input.seat.is_some() {
            // Getting into a seat isn't landing.
            self.landing = None;
            self.airborne_for = 0.0;
            self.falling = 0.0;
            return;
        }
        if !input.grounded {
            self.airborne_for += dt;
            self.falling = self.falling.max(-input.climb);
            self.landing = None;
            return;
        }
        if self.airborne_for > 0.0 {
            let l = rig.landing;
            let landed = if self.falling >= l.hard_speed {
                Some(("land_hard", l.hard_time))
            } else if self.falling >= l.soft_speed {
                Some(("land_soft", l.soft_time))
            } else {
                None
            };
            self.landing = landed.and_then(|(what, most)| {
                let a = rig.find(stance(input), input.style, what)?;
                let length = rig.duration(a);
                Some((a, if most > 0.0 { length.min(most) } else { length }))
            });
            self.airborne_for = 0.0;
            self.falling = 0.0;
        }
        let moving = input.velocity.length() >= STILL;
        if moving || self.landing.is_some_and(|(_, left)| left <= 0.0) {
            self.landing = None;
        }
    }

    /// The base animation for how the body moves, how fast to play it, and
    /// whether it's a walking or running stride. A stride plays at the pace
    /// that carries the body as fast as it goes that way (from how far
    /// each of its frames moves it): walking where that's nearer its own
    /// pace than running's.
    fn movement(&self, rig: &BodyRig, input: &BodyInput) -> (Option<usize>, f32, bool) {
        let find = |what: &str| rig.find(stance(input), input.style, what);
        if let Some((landing, _)) = self.landing {
            return (Some(landing), 1.0, false);
        }
        if !input.grounded && self.airborne_for >= AIRBORNE_AFTER {
            return (find("airborne"), 1.0, false);
        }
        let v = input.velocity;
        let speed = v.length();
        if speed < STILL {
            return (find("idle"), 1.0, false);
        }
        let way = if v.x.abs() >= v.y.abs() {
            if v.x > 0.0 {
                "front"
            } else {
                "back"
            }
        } else if v.y > 0.0 {
            "left"
        } else {
            "right"
        };
        // How fast the body goes the animation's way, and the animation's
        // own pace.
        let paces = |a: usize| {
            let own = rig.speed(a);
            let pace = own.length();
            (pace > 0.05).then(|| (v.dot(own / pace), pace))
        };
        let run = find(&format!("move_{way}"));
        let walk = find(&format!("walk_{way}"));
        let picked = match (run.and_then(|r| Some((r, paces(r)?))), walk) {
            (Some((r, (along, pace))), Some(w)) => match paces(w) {
                // Nearer walking's pace than running's (in proportion).
                Some((_, walk_pace)) if along * along < pace * walk_pace => {
                    Some((w, along / walk_pace))
                }
                _ => Some((r, along / pace)),
            },
            (Some((r, (along, pace))), None) => Some((r, along / pace)),
            // Not saying how fast it goes: played at its pace at a run.
            (None, _) => run.map(|r| (r, (speed / RUN_SPEED).clamp(0.5, 1.5))),
        };
        match picked {
            Some((a, rate)) => (Some(a), rate.clamp(MOVE_RATES.0, MOVE_RATES.1), true),
            None => (None, 1.0, false),
        }
    }
}

fn stance(input: &BodyInput) -> &'static str {
    if let Some(seat) = input.seat {
        seat
    } else if input.crouching {
        "crouch"
    } else {
        "combat"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blam_cache::animation::{Animation, GraphNode, Track};
    use blam_cache::model::Node;

    fn anim(name: &str, kind: AnimationKind, z: f32) -> Animation {
        Animation {
            name: name.into(),
            kind,
            frame_count: 2,
            decoded: true,
            rotations: vec![None, None],
            translations: vec![
                Some(Track {
                    frames: vec![0, 1],
                    values: vec![[0.0, 0.0, z]; 2],
                }),
                None,
            ],
            scales: vec![None, None],
            sound_events: Vec::new(),
            frame_events: Vec::new(),
            movement: Vec::new(),
        }
    }

    /// A stride `frames` long whose frames each carry the body `step`
    /// (forward, left), its left foot down at frame `foot`.
    fn stride(name: &str, frames: u16, step: [f32; 2], foot: u16) -> Animation {
        Animation {
            frame_count: frames,
            movement: vec![[step[0], step[1], 0.0]; frames as usize],
            frame_events: vec![(foot, FrameEvent::LeftFoot)],
            ..anim(name, AnimationKind::Base, 0.38)
        }
    }

    fn rig() -> BodyRig {
        let node = |name: &str, parent: i16| Node {
            name: name.into(),
            parent,
            translation: [0.0, 0.0, 0.5],
            rotation: [0.0, 0.0, 0.0, 1.0],
        };
        let skeleton = Skeleton::new(&[node("pelvis", -1), node("head", 0)]);
        let graph = AnimationGraph {
            parent: None,
            sounds: Vec::new(),
            nodes: vec![
                GraphNode {
                    name: "pelvis".into(),
                    parent: -1,
                },
                GraphNode {
                    name: "head".into(),
                    parent: 0,
                },
            ],
            animations: vec![
                anim("combat:rifle:idle", AnimationKind::Base, 0.4),
                anim(
                    "combat:pistol:hp:reload_1:var1",
                    AnimationKind::Replacement,
                    0.3,
                ),
                anim("crouch:rifle:idle", AnimationKind::Base, 0.2),
                // The Spartan's: 2.26 and 1.91 a second running, 0.36 walking.
                stride("combat:rifle:move_front", 20, [0.0754, 0.0], 1),
                stride("combat:rifle:move_left", 20, [0.0, 0.0636], 10),
                stride("combat:rifle:walk_front", 40, [0.012, 0.0], 1),
                anim("combat:rifle:airborne", AnimationKind::Base, 0.5),
                Animation {
                    frame_count: 29,
                    ..anim("combat:rifle:land_soft", AnimationKind::Base, 0.3)
                },
            ],
        };
        let landing = Landing::of(&BipedPhysics::default());
        BodyRig::new(graph, skeleton, SkinnedMesh::default(), [None; 2], landing)
    }

    fn standing() -> BodyInput {
        BodyInput {
            velocity: Vec2::ZERO,
            climb: 0.0,
            grounded: true,
            crouching: false,
            alive: true,
            style: ("rifle", "br"),
            seat: None,
            pitch: 0.0,
        }
    }

    #[test]
    fn animation_names_fall_back_to_general_ones() {
        let r = rig();
        // No pistol idle: the rifle one.
        assert_eq!(r.find("combat", ("pistol", "hp"), "idle"), Some(0));
        // Variants answer to the name without their suffix.
        assert_eq!(r.find("crouch", ("pistol", "hp"), "reload_1"), Some(1));
        assert_eq!(r.find("crouch", ("rifle", "br"), "idle"), Some(2));
        assert_eq!(r.find("combat", ("rifle", "br"), "dance"), None);
    }

    #[test]
    fn standing_still_plays_idle() {
        let r = rig();
        let mut a = BodyAnimator::default();
        let input = standing();
        let pose = a.update(&r, &input, 0.016);
        assert!((pose[0].translation.z - 0.4).abs() < 1e-5);
        // Looking up tips the head back.
        let up = a.update(
            &r,
            &BodyInput {
                pitch: 0.8,
                ..input
            },
            0.016,
        );
        let head = r.world(&up)[1].transform_vector3(Vec3::X);
        let level = r.world(&pose)[1].transform_vector3(Vec3::X);
        assert!(head.z > level.z + 0.1, "{head} vs {level}");
        let crouched = BodyInput {
            crouching: true,
            ..input
        };
        for _ in 0..30 {
            a.update(&r, &crouched, 0.016);
        }
        let pose = a.update(&r, &crouched, 0.016);
        assert!((pose[0].translation.z - 0.2).abs() < 1e-5);
    }

    /// Strides play at the pace their own movement says, so the feet
    /// keep up with the ground: strafing at 2.0 plays the 1.91 a second
    /// `move_left` a little fast, and creeping along at 0.4 walks.
    #[test]
    fn strides_play_at_the_pace_the_body_moves() {
        let r = rig();
        let a = BodyAnimator::default();
        let moving = |x: f32, y: f32| BodyInput {
            velocity: Vec2::new(x, y),
            ..standing()
        };
        let (anim, rate, striding) = a.movement(&r, &moving(2.25, 0.0));
        assert_eq!((anim, striding), (Some(3), true));
        assert!((rate - 2.25 / 2.262).abs() < 0.01, "{rate}");
        let (anim, rate, _) = a.movement(&r, &moving(0.0, 2.0));
        assert_eq!(anim, Some(4));
        assert!((rate - 2.0 / 1.908).abs() < 0.01, "{rate}");
        let (anim, rate, _) = a.movement(&r, &moving(0.4, 0.0));
        assert_eq!(anim, Some(5), "walks");
        assert!((rate - 0.4 / 0.36).abs() < 0.01, "{rate}");
        // Diagonally: the way it goes most, at the pace it goes that way.
        let (anim, rate, _) = a.movement(&r, &moving(1.6, 1.4));
        assert_eq!(anim, Some(3));
        assert!((rate - 1.6 / 2.262).abs() < 0.01, "{rate}");
    }

    /// Turning from running forward to sideways carries on the stride
    /// from the same foot instead of starting it again.
    #[test]
    fn changing_direction_keeps_the_stride() {
        let r = rig();
        // A quarter of the way round from the left foot (frame 1).
        let time = (1.0 + 19.0 * 0.25) / FRAME_RATE;
        let carried = r.same_stride(3, time, 4);
        let expected = (10.0 + 19.0 * 0.25) / FRAME_RATE;
        assert!((carried - expected).abs() < 1e-4, "{carried} {expected}");
        let mut a = BodyAnimator::default();
        let forward = BodyInput {
            velocity: Vec2::new(2.25, 0.0),
            ..standing()
        };
        for _ in 0..10 {
            a.update(&r, &forward, 1.0 / 60.0);
        }
        let before = a.base_time;
        a.update(
            &r,
            &BodyInput {
                velocity: Vec2::new(0.0, 2.0),
                ..forward
            },
            1.0 / 60.0,
        );
        assert_eq!(a.base, Some(4));
        assert!(a.base_time > before, "{} after {before}", a.base_time);
    }

    /// A bump doesn't show as being in the air; a jump does, and coming
    /// down from it bends the knees.
    #[test]
    fn jumps_land_softly_and_bumps_dont_show() {
        let r = rig();
        let mut a = BodyAnimator::default();
        let dt = 1.0 / 60.0;
        let air = BodyInput {
            grounded: false,
            climb: -0.3,
            ..standing()
        };
        for _ in 0..3 {
            a.update(&r, &air, dt);
        }
        assert_eq!(a.base, Some(0), "still standing after three ticks");
        a.update(&r, &standing(), dt);
        assert_eq!(a.base, Some(0), "too slow for a landing");
        for k in 0..60 {
            let climb = 3.0 - 6.0 * k as f32 / 59.0;
            a.update(&r, &BodyInput { climb, ..air }, dt);
        }
        assert_eq!(a.base, Some(6), "in the air");
        a.update(&r, &standing(), dt);
        assert_eq!(a.base, Some(7), "lands softly");
        // For no longer than the tag's longest soft landing.
        for _ in 0..40 {
            a.update(&r, &standing(), dt);
        }
        assert_eq!(a.base, Some(0));
        // Running on landing goes straight on running.
        for _ in 0..60 {
            a.update(&r, &BodyInput { climb: -3.0, ..air }, dt);
        }
        let running = BodyInput {
            velocity: Vec2::new(2.25, 0.0),
            ..standing()
        };
        a.update(&r, &running, dt);
        assert_eq!(a.base, Some(3));
    }

    /// Killed on the way down, or getting into a seat, the body doesn't
    /// land from that fall afterwards.
    #[test]
    fn no_landing_after_dying_or_sitting_down() {
        let r = rig();
        let dt = 1.0 / 60.0;
        let falling = BodyInput {
            grounded: false,
            climb: -3.0,
            ..standing()
        };
        let dead = BodyInput {
            alive: false,
            ..standing()
        };
        let seated = BodyInput {
            seat: Some("warthog_p"),
            ..standing()
        };
        for then in [dead, seated] {
            let mut a = BodyAnimator::default();
            for _ in 0..60 {
                a.update(&r, &falling, dt);
            }
            a.update(&r, &then, dt);
            a.update(&r, &standing(), dt);
            assert_eq!(a.base, Some(0), "stands after {then:?}");
        }
    }
}
