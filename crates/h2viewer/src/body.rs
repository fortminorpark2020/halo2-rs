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
use blam_cache::animation::{AnimationGraph, AnimationKind, FRAME_RATE};
use blam_cache::model::Marker;
use glam::{Mat4, Quat, Vec2, Vec3};
use std::collections::HashMap;

/// Seconds to fade between base animations.
const BASE_BLEND: f32 = 0.2;
/// Seconds to fade actions in and out.
const ACTION_FADE: f32 = 0.1;
/// Run speed at which movement animations play at their own pace.
const RUN_SPEED: f32 = 2.25;

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
    previous: Option<(usize, f32)>,
    blend: f32,
    action: Option<usize>,
    action_time: f32,
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

    /// Advance by `dt` and pose the body.
    pub fn update(&mut self, rig: &BodyRig, input: &BodyInput, dt: f32) -> Vec<NodePose> {
        let (want, rate) = if !input.alive {
            (rig.by_name("combat:landing_dead"), 1.0)
        } else {
            let speed = input.velocity.length();
            let state = if !input.grounded {
                "airborne"
            } else if speed < 0.15 {
                "idle"
            } else if input.velocity.x.abs() >= input.velocity.y.abs() {
                if input.velocity.x > 0.0 {
                    "move_front"
                } else {
                    "move_back"
                }
            } else if input.velocity.y > 0.0 {
                "move_left"
            } else {
                "move_right"
            };
            let rate = if state.starts_with("move") {
                (speed / RUN_SPEED).clamp(0.5, 1.5)
            } else {
                1.0
            };
            (rig.find(stance(input), input.style, state), rate)
        };
        if want != self.base {
            self.previous = self.base.map(|b| (b, self.base_time));
            self.base = want;
            self.base_time = 0.0;
            self.blend = 0.0;
            if !input.alive {
                self.action = None;
            }
        }
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

    fn rig() -> BodyRig {
        let node = |name: &str, parent: i16| Node {
            name: name.into(),
            parent,
            translation: [0.0, 0.0, 0.5],
            rotation: [0.0, 0.0, 0.0, 1.0],
        };
        let skeleton = Skeleton::new(&[node("pelvis", -1), node("head", 0)]);
        let anim = |name: &str, kind: AnimationKind, z: f32| Animation {
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
        };
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
            ],
        };
        BodyRig::new(graph, skeleton, SkinnedMesh::default(), [None; 2])
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
        let input = BodyInput {
            velocity: Vec2::ZERO,
            grounded: true,
            crouching: false,
            alive: true,
            style: ("rifle", "br"),
            seat: None,
            pitch: 0.0,
        };
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
}
