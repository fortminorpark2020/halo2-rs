//! Campaign machines that move whoever stands on them (lifts, elevators,
//! the tram), posed by their `device:position` animation, and the switches
//! players use.

use super::{Loader, MeshData};
use crate::rig::{tag_quat, world_matrices, NodePose};
use blam_cache::animation::{self, Track};
use blam_cache::model;
use blam_cache::scenario::{self, ControlKind, Placement};
use glam::{Mat4, Vec3};

/// The `mach` type of lifts (doors are 0, gears 2).
const PLATFORM: u16 = 1;
/// Lifts bigger than this (a capital ship flying by) don't collide.
const MAX_SOLID_TRIANGLES: usize = 20_000;

/// A machine that carries what stands on it, moving from where it was
/// placed (position 0) to where its animation ends (1).
pub struct Lift {
    pub name: Option<u16>,
    /// In the level from the start (otherwise a script creates it).
    pub placed: bool,
    /// At the end of its travel from the start.
    pub open: bool,
    pub powered: bool,
    /// The device group it follows, if any.
    pub position_group: Option<u16>,
    /// Seconds for the whole way.
    pub time: f32,
    pub transform: Mat4,
    pub light: Option<[f32; 3]>,
    parents: Vec<i16>,
    rest: Vec<NodePose>,
    /// The animation's tracks, by the render model's nodes.
    rotations: Vec<Option<Track<[f32; 4]>>>,
    translations: Vec<Option<Track<[f32; 3]>>>,
    frames: u16,
    pub parts: Vec<LiftPart>,
}

/// The part of a lift one of its nodes moves.
pub struct LiftPart {
    pub mesh: usize,
    pub node: usize,
    /// Its collision in the world (`World::add_mover`), unless it's too big
    /// to bother.
    pub mover: Option<usize>,
    /// Where it starts, in the world.
    pub triangles: Vec<[Vec3; 3]>,
    centre: Vec3,
}

impl Lift {
    /// How each node has moved, in the world, at a position from 0 to 1.
    pub fn poses(&self, position: f32) -> Vec<Mat4> {
        let frame = position.clamp(0.0, 1.0) * self.frames.saturating_sub(1) as f32;
        let posed: Vec<NodePose> = self
            .rest
            .iter()
            .enumerate()
            .map(|(n, rest)| {
                let mut p = *rest;
                // The animation is an overlay: it adds to the rest pose.
                if let Some(t) = &self.translations[n] {
                    p.translation += Vec3::from(t.sample(frame));
                }
                if let Some(r) = &self.rotations[n] {
                    p.rotation = (rest.rotation * tag_quat(r.sample(frame))).normalize();
                }
                p
            })
            .collect();
        let rest = world_matrices(&self.parents, &self.rest);
        let posed = world_matrices(&self.parents, &posed);
        let inverse = self.transform.inverse();
        rest.iter()
            .zip(posed)
            .map(|(r, p)| self.transform * p * r.inverse() * inverse)
            .collect()
    }

    /// How far a part has moved at a pose.
    pub fn offset(&self, poses: &[Mat4], part: &LiftPart) -> Vec3 {
        poses[part.node].transform_point3(part.centre) - part.centre
    }
}

/// A switch players use: it sets a device group (or calls the lift next
/// to it).
pub struct Switch {
    pub name: Option<u16>,
    pub placed: bool,
    pub position: Vec3,
    pub kind: ControlKind,
    pub call_value: f32,
    pub position_group: Option<u16>,
    pub powered: bool,
}

impl Switch {
    pub fn new(set: &mut blam_cache::MapSet, p: &Placement) -> Switch {
        let control = scenario::control(set, p.object).unwrap_or_default();
        Switch {
            name: p.name,
            placed: p.automatic,
            position: Vec3::from(p.position),
            kind: control.kind,
            call_value: control.call_value,
            position_group: p.position_group,
            powered: p.device_flags & 2 == 0,
        }
    }
}

impl MeshData {
    /// The triangles each node carries, as meshes of their own (by the
    /// node of a triangle's first corner).
    fn split_by_node(&self) -> Vec<(usize, MeshData)> {
        let node_of = |i: u32| self.bones.get(i as usize).map_or(0, |b| b[0] as usize);
        let mut nodes: Vec<usize> = self.indices.iter().map(|&i| node_of(i)).collect();
        nodes.sort_unstable();
        nodes.dedup();
        nodes
            .into_iter()
            .map(|n| {
                let mut mesh = MeshData {
                    vertices: self.vertices.clone(),
                    bones: self.bones.clone(),
                    weights: self.weights.clone(),
                    baked_lighting: self.baked_lighting,
                    ..MeshData::default()
                };
                for b in &self.batches {
                    let range = b.first_index as usize..(b.first_index + b.index_count) as usize;
                    let first = mesh.indices.len() as u32;
                    for t in self.indices[range].as_chunks::<3>().0 {
                        if node_of(t[0]) == n {
                            mesh.indices.extend_from_slice(t);
                        }
                    }
                    let count = mesh.indices.len() as u32 - first;
                    if count > 0 {
                        mesh.batches.push(super::Batch {
                            first_index: first,
                            index_count: count,
                            ..*b
                        });
                    }
                }
                (n, mesh)
            })
            .collect()
    }
}

impl Loader {
    /// A placed machine as a lift, if it's a platform with an animation to
    /// move it by. Its parts' meshes go in `meshes`; `movers` counts the
    /// world's movers so far.
    pub(super) fn lift(
        &mut self,
        p: &Placement,
        mesh: usize,
        meshes: &mut Vec<MeshData>,
        light: Option<[f32; 3]>,
        movers: &mut usize,
    ) -> Option<Lift> {
        let machine = scenario::machine(&mut self.set, p.object).ok()?;
        if machine.kind != PLATFORM {
            return None;
        }
        let graph = model::object_animations(&mut self.set, p.object)
            .and_then(|jmad| animation::read_animation_graph(&mut self.set, jmad))
            .ok()?;
        let anim = graph.find("device:position")?;
        let render = model::read_object_render_model(&mut self.set, p.object).ok()?;
        let by_graph = |n: &model::Node| graph.node(&n.name);
        let transform = super::placement_matrix(p.position, p.rotation, p.scale);
        let parts: Vec<(usize, MeshData)> = meshes[mesh].split_by_node();
        let solid = meshes[mesh].triangle_count() <= MAX_SOLID_TRIANGLES;
        let parts = parts
            .into_iter()
            .map(|(node, m)| {
                let triangles: Vec<[Vec3; 3]> = m
                    .indices
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|t| {
                        t.map(|i| {
                            transform.transform_point3(Vec3::from(m.vertices[i as usize].position))
                        })
                    })
                    .collect();
                let centre = triangles.iter().flatten().copied().sum::<Vec3>()
                    / (triangles.len() * 3).max(1) as f32;
                meshes.push(m);
                let mover = solid.then(|| {
                    *movers += 1;
                    *movers - 1
                });
                LiftPart {
                    mesh: meshes.len() - 1,
                    node: node.min(render.nodes.len().saturating_sub(1)),
                    mover,
                    triangles,
                    centre,
                }
            })
            .collect();
        Some(Lift {
            name: p.name,
            placed: p.automatic,
            open: p.device_flags & 1 != 0,
            powered: p.device_flags & 2 == 0,
            position_group: p.position_group,
            time: if machine.position_time > 0.0 {
                machine.position_time
            } else {
                1.0
            },
            transform,
            light,
            parents: render.nodes.iter().map(|n| n.parent).collect(),
            rest: render
                .nodes
                .iter()
                .map(|n| NodePose {
                    rotation: tag_quat(n.rotation),
                    translation: Vec3::from(n.translation),
                    scale: 1.0,
                })
                .collect(),
            rotations: render
                .nodes
                .iter()
                .map(|n| by_graph(n).and_then(|g| anim.rotations.get(g).cloned().flatten()))
                .collect(),
            translations: render
                .nodes
                .iter()
                .map(|n| by_graph(n).and_then(|g| anim.translations.get(g).cloned().flatten()))
                .collect(),
            frames: anim.frame_count,
            parts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    fn lift(translation: Track<[f32; 3]>, transform: Mat4) -> Lift {
        Lift {
            name: None,
            placed: true,
            open: false,
            powered: true,
            position_group: None,
            time: 1.0,
            transform,
            light: None,
            parents: vec![-1, 0],
            rest: vec![NodePose::IDENTITY; 2],
            rotations: vec![None, None],
            translations: vec![Some(translation), None],
            frames: 11,
            parts: Vec::new(),
        }
    }

    #[test]
    fn a_lift_moves_its_nodes_by_its_animation_turned_as_placed() {
        let track = Track {
            frames: vec![0, 10],
            values: vec![[0.0; 3], [2.0, 0.0, 4.0]],
        };
        // Turned a quarter left: its +x is the world's +y.
        let turned = Mat4::from_rotation_translation(
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Vec3::new(10.0, 0.0, 0.0),
        );
        let l = lift(track, turned);
        let at = |p: f32, node: usize| l.poses(p)[node].transform_point3(Vec3::new(10.0, 0.0, 0.0));
        assert!(at(0.0, 0).abs_diff_eq(Vec3::new(10.0, 0.0, 0.0), 1e-4));
        assert!(at(0.5, 0).abs_diff_eq(Vec3::new(10.0, 1.0, 2.0), 1e-4));
        assert!(
            at(1.0, 1).abs_diff_eq(Vec3::new(10.0, 2.0, 4.0), 1e-4),
            "children follow"
        );
    }
}
