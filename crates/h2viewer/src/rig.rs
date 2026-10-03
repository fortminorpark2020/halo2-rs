//! Posing skinned models with Halo 2's animations: the first person arms and
//! the gun they hold.
//!
//! A first person animation graph animates one skeleton made of the arms'
//! nodes, the gun's nodes (under the right hand) and a `camera_control` node.
//! Nodes are matched to the models' nodes by name; whatever an animation
//! leaves alone keeps the model's default pose.

use crate::scene::{MeshData, Vertex};
use blam_cache::animation::{Animation, AnimationGraph, AnimationKind, FRAME_RATE};
use blam_cache::model::Node;
use glam::{Mat4, Quat, Vec3};

/// Seconds to cross-fade from one animation into the next.
const BLEND_TIME: f32 = 0.12;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodePose {
    pub rotation: Quat,
    pub translation: Vec3,
    pub scale: f32,
}

impl NodePose {
    pub const IDENTITY: NodePose = NodePose {
        rotation: Quat::IDENTITY,
        translation: Vec3::ZERO,
        scale: 1.0,
    };

    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(
            Vec3::splat(self.scale),
            self.rotation,
            self.translation,
        )
    }

    pub fn lerp(&self, to: &NodePose, t: f32) -> NodePose {
        NodePose {
            rotation: self.rotation.slerp(to.rotation, t),
            translation: self.translation.lerp(to.translation, t),
            scale: self.scale + (to.scale - self.scale) * t,
        }
    }
}

/// A tag quaternion (i, j, k, w): a node's rotation relative to its parent.
pub fn tag_quat(q: [f32; 4]) -> Quat {
    Quat::from_xyzw(q[0], q[1], q[2], q[3]).normalize()
}

/// World matrices of a hierarchy whose parents come before their children.
pub fn world_matrices(parents: &[i16], local: &[NodePose]) -> Vec<Mat4> {
    let mut world: Vec<Mat4> = Vec::with_capacity(local.len());
    for (i, pose) in local.iter().enumerate() {
        let m = pose.matrix();
        let w = match usize::try_from(parents[i]).ok().filter(|&p| p < i) {
            Some(p) => world[p] * m,
            None => m,
        };
        world.push(w);
    }
    world
}

/// A render model's node hierarchy in its default (bind) pose.
#[derive(Clone, Debug, Default)]
pub struct Skeleton {
    pub names: Vec<String>,
    pub default: Vec<NodePose>,
    pub inverse_bind: Vec<Mat4>,
}

impl Skeleton {
    pub fn new(nodes: &[Node]) -> Skeleton {
        let default: Vec<NodePose> = nodes
            .iter()
            .map(|n| NodePose {
                rotation: tag_quat(n.rotation),
                translation: Vec3::from(n.translation),
                scale: 1.0,
            })
            .collect();
        let parents: Vec<i16> = nodes.iter().map(|n| n.parent).collect();
        let inverse_bind = world_matrices(&parents, &default)
            .iter()
            .map(Mat4::inverse)
            .collect();
        Skeleton {
            names: nodes.iter().map(|n| n.name.clone()).collect(),
            default,
            inverse_bind,
        }
    }

    pub fn node(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }
}

/// A skinned mesh's rest vertices, kept on the CPU so they can be posed.
#[derive(Clone, Default)]
pub struct SkinnedMesh {
    pub rest: Vec<Vertex>,
    pub bones: Vec<[u8; 4]>,
    pub weights: Vec<[f32; 4]>,
}

impl SkinnedMesh {
    pub fn new(mesh: &MeshData) -> SkinnedMesh {
        SkinnedMesh {
            rest: mesh.vertices.clone(),
            bones: mesh.bones.clone(),
            weights: mesh.weights.clone(),
        }
    }

    /// The vertices posed by one matrix per model node.
    pub fn pose(&self, skin: &[Mat4]) -> Vec<Vertex> {
        self.rest
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let (bones, weights) = (self.bones[i], self.weights[i]);
                let mut m = Mat4::ZERO;
                for k in 0..4 {
                    if weights[k] > 0.0 {
                        let s = skin
                            .get(bones[k] as usize)
                            .copied()
                            .unwrap_or(Mat4::IDENTITY);
                        m += s * weights[k];
                    }
                }
                Vertex {
                    position: m.transform_point3(Vec3::from(v.position)).into(),
                    normal: m
                        .transform_vector3(Vec3::from(v.normal))
                        .normalize_or(Vec3::Z)
                        .into(),
                    ..*v
                }
            })
            .collect()
    }
}

/// One weapon's first person animations, tied to the arms and gun models.
pub struct FirstPersonRig {
    pub graph: AnimationGraph,
    /// Pose of each graph node when no animation moves it.
    defaults: Vec<NodePose>,
    /// For each arms / gun model node, the graph node driving it.
    arms_nodes: Vec<Option<usize>>,
    gun_nodes: Vec<Option<usize>>,
    camera: Option<usize>,
}

impl FirstPersonRig {
    pub fn new(graph: AnimationGraph, arms: &Skeleton, gun: &Skeleton) -> FirstPersonRig {
        let defaults = graph
            .nodes
            .iter()
            .map(|n| {
                if let Some(i) = gun.node(&n.name) {
                    gun.default[i]
                } else if let Some(i) = arms.node(&n.name) {
                    arms.default[i]
                } else {
                    NodePose::IDENTITY
                }
            })
            .collect();
        let arms_nodes = arms.names.iter().map(|n| graph.node(n)).collect();
        let gun_nodes = gun.names.iter().map(|n| graph.node(n)).collect();
        let camera = graph.node("camera_control");
        FirstPersonRig {
            defaults,
            arms_nodes,
            gun_nodes,
            camera,
            graph,
        }
    }

    /// Index of the first animation whose name starts with `prefix`; with
    /// several variants (`fire_1:var1`, `:var2`...) `pick` chooses among them.
    pub fn find(&self, prefix: &str, pick: usize) -> Option<usize> {
        let matches: Vec<usize> = self
            .graph
            .animations
            .iter()
            .enumerate()
            .filter(|(_, a)| a.decoded && a.kind == AnimationKind::Base)
            .filter(|(_, a)| a.name == prefix || a.name.starts_with(&format!("{prefix}:")))
            .map(|(i, _)| i)
            .collect();
        (!matches.is_empty()).then(|| matches[pick % matches.len()])
    }

    pub fn sample(&self, anim: &Animation, frame: f32) -> Vec<NodePose> {
        (0..self.defaults.len())
            .map(|n| {
                let d = self.defaults[n];
                NodePose {
                    rotation: anim.rotations[n]
                        .as_ref()
                        .map_or(d.rotation, |t| tag_quat(t.sample(frame))),
                    translation: anim.translations[n]
                        .as_ref()
                        .map_or(d.translation, |t| Vec3::from(t.sample(frame))),
                    scale: anim.scales[n].as_ref().map_or(d.scale, |t| t.sample(frame)),
                }
            })
            .collect()
    }

    pub fn rest_pose(&self) -> Vec<NodePose> {
        self.defaults.clone()
    }

    /// Graph node world matrices, placed so the camera node sits at the origin
    /// looking down +x.
    pub fn world(&self, pose: &[NodePose]) -> Vec<Mat4> {
        let parents: Vec<i16> = self.graph.nodes.iter().map(|n| n.parent).collect();
        let world = world_matrices(&parents, pose);
        let root = self
            .camera
            .and_then(|c| world.get(c))
            .map_or(Mat4::IDENTITY, Mat4::inverse);
        world.into_iter().map(|w| root * w).collect()
    }

    /// Skinning matrices for a model whose nodes map through `nodes`.
    fn skin(&self, world: &[Mat4], skeleton: &Skeleton, nodes: &[Option<usize>]) -> Vec<Mat4> {
        (0..skeleton.names.len())
            .map(|i| {
                let w = nodes[i].and_then(|g| world.get(g)).copied();
                w.map_or(Mat4::IDENTITY, |w| w * skeleton.inverse_bind[i])
            })
            .collect()
    }

    /// World matrix of a gun model node, if the animation drives it.
    pub fn gun_node_world(&self, world: &[Mat4], node: usize) -> Option<Mat4> {
        self.gun_nodes
            .get(node)
            .copied()
            .flatten()
            .and_then(|g| world.get(g))
            .copied()
    }

    pub fn arms_skin(&self, world: &[Mat4], arms: &Skeleton) -> Vec<Mat4> {
        self.skin(world, arms, &self.arms_nodes)
    }

    pub fn gun_skin(&self, world: &[Mat4], gun: &Skeleton) -> Vec<Mat4> {
        self.skin(world, gun, &self.gun_nodes)
    }
}

/// Plays one first person animation at a time, fading between them.
#[derive(Default)]
pub struct Animator {
    anim: Option<usize>,
    time: f32,
    looping: bool,
    blend_from: Vec<NodePose>,
    blend: f32,
    pose: Vec<NodePose>,
}

impl Animator {
    /// The latest pose (graph node local transforms); empty before the first update.
    pub fn pose(&self) -> &[NodePose] {
        &self.pose
    }

    pub fn play(&mut self, anim: Option<usize>, looping: bool) {
        self.blend_from = self.pose.clone();
        self.blend = if self.blend_from.is_empty() {
            BLEND_TIME
        } else {
            0.0
        };
        self.anim = anim;
        self.time = 0.0;
        self.looping = looping;
    }

    /// True once a non-looping animation has played to its end.
    pub fn finished(&self, rig: &FirstPersonRig) -> bool {
        match self.anim.and_then(|a| rig.graph.animations.get(a)) {
            Some(a) => !self.looping && self.time >= a.duration(),
            None => true,
        }
    }

    pub fn update(&mut self, rig: &FirstPersonRig, dt: f32) -> &[NodePose] {
        self.time += dt;
        self.blend += dt;
        let pose = match self.anim.and_then(|a| rig.graph.animations.get(a)) {
            Some(a) => {
                let last = a.frame_count.saturating_sub(1).max(1) as f32;
                let mut frame = self.time * FRAME_RATE;
                frame = if self.looping {
                    frame % last
                } else {
                    frame.min(last)
                };
                rig.sample(a, frame)
            }
            None => rig.rest_pose(),
        };
        let t = (self.blend / BLEND_TIME).clamp(0.0, 1.0);
        self.pose = if t < 1.0 && self.blend_from.len() == pose.len() {
            self.blend_from
                .iter()
                .zip(&pose)
                .map(|(a, b)| a.lerp(b, t))
                .collect()
        } else {
            pose
        };
        &self.pose
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, parent: i16, t: [f32; 3]) -> Node {
        Node {
            name: name.into(),
            parent,
            translation: t,
            rotation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    #[test]
    fn hierarchy_accumulates_translations() {
        let s = Skeleton::new(&[
            node("a", -1, [1.0, 0.0, 0.0]),
            node("b", 0, [0.0, 2.0, 0.0]),
        ]);
        let w = world_matrices(&[-1, 0], &s.default);
        assert_eq!(w[1].transform_point3(Vec3::ZERO), Vec3::new(1.0, 2.0, 0.0));
        // The bind pose skins every vertex to where it already is.
        let skin = w[1] * s.inverse_bind[1];
        assert!(skin.abs_diff_eq(Mat4::IDENTITY, 1e-6));
    }

    #[test]
    fn skinning_blends_by_weight() {
        let mesh = SkinnedMesh {
            rest: vec![Vertex::new([0.0; 3], [0.0, 0.0, 1.0], [0.0; 2])],
            bones: vec![[0, 1, 0, 0]],
            weights: vec![[0.5, 0.5, 0.0, 0.0]],
        };
        let skin = [
            Mat4::from_translation(Vec3::X),
            Mat4::from_translation(Vec3::Y),
        ];
        assert_eq!(mesh.pose(&skin)[0].position, [0.5, 0.5, 0.0]);
    }
}
