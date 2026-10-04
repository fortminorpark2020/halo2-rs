//! Vehicles from the map's tags: their models, how they drive, their seats
//! and guns, and the turrets their variants mount.

use super::{Loader, MeshData, Vertex, WeaponAssets};
use crate::rig::{tag_quat, world_matrices, NodePose, Skeleton, SkinnedMesh};
use blam_cache::model::{self, RenderModel};
use blam_cache::vehicle::{self, seat_flags, FrictionPoint, VehicleTag, VehicleType};
use blam_cache::{scenario, DatumIndex, GroupTag};
use glam::{Mat3, Mat4, Quat, Vec3};
use h2sim::game::VehicleSpawn;
use h2sim::vehicle::{
    Drive, HoverPad, HullBox, SeatDef, SeatRole, Vehicle, VehicleDef, Wheel, SUSPENSION_TRAVEL,
};
use std::collections::HashMap;

/// A posable vehicle model: the vehicle itself, or its turret.
pub struct VehiclePart {
    pub skeleton: Skeleton,
    pub parents: Vec<i16>,
    pub skin: SkinnedMesh,
    /// Nodes that turn with the turret's aim, around and up.
    pub yaw_node: Option<usize>,
    pub pitch_node: Option<usize>,
}

impl VehiclePart {
    fn new(m: &RenderModel, mesh: &MeshData) -> VehiclePart {
        let node = |names: &[&str]| {
            names
                .iter()
                .find_map(|n| m.nodes.iter().position(|x| x.name == *n))
        };
        VehiclePart {
            skeleton: Skeleton::new(&m.nodes),
            parents: m.nodes.iter().map(|n| n.parent).collect(),
            skin: SkinnedMesh::new(mesh),
            yaw_node: node(&["gun_mount_base", "arm"]),
            pitch_node: node(&["gun"]),
        }
    }

    /// The model posed: each node's default, changed by `adjust`.
    pub fn pose(&self, adjust: impl Fn(usize, &mut NodePose)) -> Vec<Vertex> {
        let mut local = self.skeleton.default.clone();
        for (i, p) in local.iter_mut().enumerate() {
            adjust(i, p);
        }
        let world = world_matrices(&self.parents, &local);
        let skin: Vec<Mat4> = world
            .iter()
            .zip(&self.skeleton.inverse_bind)
            .map(|(w, inv)| *w * *inv)
            .collect();
        self.skin.pose(&skin)
    }

    /// Turn the aiming nodes to the turret's aim (yaw, pitch).
    fn aim(&self, i: usize, p: &mut NodePose, aim: glam::Vec2) {
        if Some(i) == self.yaw_node {
            p.rotation *= Quat::from_rotation_z(aim.x);
        }
        if Some(i) == self.pitch_node {
            p.rotation *= Quat::from_rotation_y(-aim.y);
        }
    }
}

/// How to draw one kind of vehicle.
pub struct VehicleAssets {
    /// What the HUD calls it.
    pub name: String,
    pub body: VehiclePart,
    /// A turret the vehicle mounts, and where it sits in the vehicle's
    /// model.
    pub turret: Option<(VehiclePart, Mat4)>,
    /// Per wheel: the tyre node (rolls) and the hub node (steers and rides
    /// the suspension).
    pub wheel_nodes: Vec<(Option<usize>, Option<usize>)>,
    pub steering_wheel: Option<usize>,
    /// Each seat's rider animations ("warthog_d"...).
    pub seat_stances: Vec<&'static str>,
    /// Getting into each seat (in `Scene::sounds`).
    pub enter_sounds: Vec<Option<usize>>,
    /// The engine running, and boosting (looped).
    pub engine: Option<usize>,
    pub boost: Option<usize>,
}

impl VehicleAssets {
    /// The vehicle's (and turret's) vertices posed for its state.
    pub fn pose(&self, def: &VehicleDef, v: &Vehicle) -> (Vec<Vertex>, Option<Vec<Vertex>>) {
        let travel = 2.0 * SUSPENSION_TRAVEL;
        let body = self.body.pose(|i, p| {
            for (k, &(tyre, hub)) in self.wheel_nodes.iter().enumerate() {
                if Some(i) == tyre {
                    p.rotation *= Quat::from_rotation_y(v.roll);
                }
                if Some(i) == hub {
                    let w = &def.wheels[k];
                    let c = v.compression.get(k).copied().unwrap_or(0.5);
                    p.translation.z += (c - 0.5) * travel;
                    if w.steers {
                        p.rotation = Quat::from_rotation_z(v.steer) * p.rotation;
                    }
                }
            }
            if Some(i) == self.steering_wheel {
                p.rotation *= Quat::from_rotation_x(-v.steer * 2.0);
            }
            if self.turret.is_none() {
                self.body.aim(i, p, v.aim);
            }
        });
        let turret = self
            .turret
            .as_ref()
            .map(|(t, _)| t.pose(|i, p| t.aim(i, p, v.aim)));
        (body, turret)
    }
}

/// Every node's default placement in the model.
fn bind_pose(m: &RenderModel) -> Vec<Mat4> {
    let local: Vec<NodePose> = m
        .nodes
        .iter()
        .map(|n| NodePose {
            rotation: tag_quat(n.rotation),
            translation: Vec3::from(n.translation),
            scale: 1.0,
        })
        .collect();
    let parents: Vec<i16> = m.nodes.iter().map(|n| n.parent).collect();
    world_matrices(&parents, &local)
}

/// A marker's placement in the model.
fn marker(m: &RenderModel, bind: &[Mat4], name: &str) -> Option<Mat4> {
    if name.is_empty() {
        return None;
    }
    let mk = m.marker(name)?;
    let node = bind
        .get(mk.node as usize)
        .copied()
        .unwrap_or(Mat4::IDENTITY);
    Some(node * Mat4::from_rotation_translation(tag_quat(mk.rotation), mk.translation.into()))
}

/// The tyre and hub nodes of a wheel ("left_front_tire": "lf_tire",
/// "lf_engine").
fn wheel_nodes(m: &RenderModel, f: &FrictionPoint) -> (Option<usize>, Option<usize>) {
    let side = if f.marker.contains("left") { "l" } else { "r" };
    let end = if f.marker.contains("front") { "f" } else { "b" };
    let find = |what: &str| {
        let name = format!("{side}{end}_{what}");
        m.nodes.iter().position(|n| n.name == name)
    };
    (find("tire"), find("engine"))
}

/// What the HUD calls a vehicle, from its tag's name.
fn display_name(tag: &str) -> String {
    let base = tag.rsplit('\\').next().unwrap_or(tag);
    match base {
        "h_turret_ap" | "h_turret_mp" | "c_turret_ap" => "TURRET".into(),
        other => other.replace('_', " ").to_uppercase(),
    }
}

/// The seats a player can sit in (not the boarding positions).
fn player_seats(tag: &VehicleTag) -> impl Iterator<Item = &vehicle::Seat> {
    tag.seats
        .iter()
        .filter(|s| !s.has(seat_flags::BOARDING) && !s.has(seat_flags::INVALID_FOR_PLAYER))
}

impl Loader {
    /// The sound a looping sound tag (`lsnd`) loops: by Halo 2's naming,
    /// `<its folder>\<track>\loop`.
    fn looping_sound(&mut self, lsnd: DatumIndex) -> Option<usize> {
        let name = self.set.locate(lsnd)?.1.name;
        let folder = &name[..name.rfind('\\')? + 1];
        let snd = GroupTag::parse("snd!")?;
        let tag = self
            .set
            .map
            .tags
            .iter()
            .find(|t| t.group == snd && t.name.starts_with(folder) && t.name.ends_with("\\loop"))?
            .datum;
        self.sound(tag)
    }

    /// The engine and boost loops among a vehicle's attachments.
    fn engine_sounds(&mut self, tag: &VehicleTag) -> (Option<usize>, Option<usize>) {
        let lsnd = GroupTag::parse("lsnd").expect("a group tag");
        let loops: Vec<(DatumIndex, String)> = tag
            .attachments
            .iter()
            .filter_map(|&(d, _)| {
                let t = self.set.locate(d)?.1;
                (t.group == lsnd).then_some((d, t.name))
            })
            .collect();
        let pick = |want: &dyn Fn(&str) -> bool| loops.iter().find(|l| want(&l.1)).map(|l| l.0);
        let engine = pick(&|n| {
            !["boost", "contrail", "horn", "scrape"]
                .iter()
                .any(|x| n.contains(x))
        });
        let boost = pick(&|n| n.contains("boost"));
        (
            engine.and_then(|d| self.looping_sound(d)),
            boost.and_then(|d| self.looping_sound(d)),
        )
    }

    /// Master Chief's sound getting into a seat ("mc_warthog_d_enter").
    fn enter_sound(&mut self, stance: &str) -> Option<usize> {
        let base = "sound\\characters\\masterchief\\mc_";
        let short = stance.strip_suffix("_d").unwrap_or(stance);
        [
            format!("{base}{stance}_enter"),
            format!("{base}{short}_enter"),
        ]
        .iter()
        .find_map(|n| self.sound_named(n))
        .or_else(|| {
            stance
                .contains("turret")
                .then(|| self.sound_named(&format!("{base}stationary_turret")))
                .flatten()
        })
    }

    /// A vehicle weapon, loaded into `weapons` once.
    fn vehicle_weapon(
        &mut self,
        weap: DatumIndex,
        weapons: &mut Vec<WeaponAssets>,
        meshes: &mut Vec<MeshData>,
    ) -> Option<usize> {
        if let Some(i) = weapons.iter().position(|w| w.tag == weap) {
            return Some(i);
        }
        let name = self.set.locate(weap)?.1.name;
        // The Warthog's "gun" is its horn.
        if name.ends_with("horn") {
            return None;
        }
        let w = self.weapon(&name, None, meshes)?;
        weapons.push(w);
        Some(weapons.len() - 1)
    }

    /// A kind of vehicle in a model variant: how it drives and how to draw it.
    fn vehicle(
        &mut self,
        vehi: DatumIndex,
        variant: &str,
        weapons: &mut Vec<WeaponAssets>,
        meshes: &mut Vec<MeshData>,
    ) -> Option<(VehicleDef, VehicleAssets, MeshData, Option<MeshData>)> {
        let name = self.set.locate(vehi)?.1.name;
        let loaded = vehicle::read_vehicle(&mut self.set, vehi).and_then(|tag| {
            let model = vehicle::read_model(&mut self.set, tag.model)?;
            let render = model::read_render_model(&mut self.set, model.render_model)?;
            Ok((tag, model, render))
        });
        let (tag, model, render) = match loaded {
            Ok(x) => x,
            Err(e) => {
                println!("warning: vehicle {name}: {e}");
                return None;
            }
        };
        let bind = bind_pose(&render);
        let at = |name: &str| marker(&render, &bind, name).map(|m| m.w_axis.truncate());
        let physics = vehicle::read_physics_model(&mut self.set, model.physics_model).ok();
        let hull: Vec<HullBox> = physics
            .iter()
            .flat_map(|p| &p.boxes)
            .map(|b| {
                let node = usize::try_from(b.node)
                    .ok()
                    .and_then(|n| bind.get(n))
                    .copied()
                    .unwrap_or(Mat4::IDENTITY);
                let axes = Mat3::from_cols(
                    Vec3::from(b.axes[0]),
                    Vec3::from(b.axes[1]),
                    Vec3::from(b.axes[2]),
                );
                HullBox {
                    center: node.transform_point3(b.center.into()),
                    half_extents: Vec3::from(b.half_extents).abs(),
                    axes: Mat3::from_mat4(node) * axes,
                }
            })
            .collect();
        let drive = match tag.kind {
            VehicleType::AlienScout => Drive::Hover,
            VehicleType::AlienFighter | VehicleType::HumanPlane => Drive::Fly,
            VehicleType::Turret => Drive::Fixed,
            _ => Drive::Wheels,
        };
        let wheels = tag
            .friction
            .iter()
            .filter_map(|f| {
                Some(Wheel {
                    position: at(&f.marker)?,
                    radius: f.radius.max(0.1),
                    powered: f.flags & FrictionPoint::POWERED != 0,
                    steers: f.flags & FrictionPoint::FRONT_TURNING != 0,
                })
            })
            .collect();
        let strength: f32 = tag.anti_gravity.iter().map(|a| a.strength).sum();
        let pads = tag
            .anti_gravity
            .iter()
            .filter_map(|a| {
                Some(HoverPad {
                    position: at(&a.marker)?,
                    height: a.height.max(0.1),
                    share: a.strength / strength.max(1e-3),
                })
            })
            .collect();

        // Seats: the vehicle's own, then its turret's.
        let built_in = tag
            .weapons
            .first()
            .and_then(|&w| self.vehicle_weapon(w, weapons, meshes));
        let body_mesh = self.model_mesh(&render);
        let mut body = VehiclePart::new(&render, &body_mesh);
        let pivot = body
            .yaw_node
            .and_then(|n| bind.get(n))
            .map(|m| m.w_axis.truncate());
        if drive != Drive::Fixed {
            body.yaw_node = None;
            body.pitch_node = None;
        }
        let mut seats = Vec::new();
        let mut stances = Vec::new();
        for s in player_seats(&tag) {
            let Some(position) = at(&s.marker) else {
                continue;
            };
            let role = if s.has(seat_flags::DRIVER) {
                SeatRole::Driver
            } else if s.has(seat_flags::GUNNER) {
                SeatRole::Gunner
            } else {
                SeatRole::Passenger
            };
            seats.push(SeatDef {
                role,
                position,
                entry: at(&s.entry_marker).unwrap_or(position),
                entry_radius: s.entry_radius.max(0.8),
                eye: at(&s.camera_marker).unwrap_or(position + Vec3::Z * 0.55),
                exposed: !s.has(seat_flags::INVISIBLE),
                third_person: s.has(seat_flags::THIRD_PERSON_CAMERA),
                weapon: s.has(seat_flags::GUNNER).then_some(built_in).flatten(),
                pivot: (drive == Drive::Fixed).then_some(pivot).flatten(),
                pitch_range: s.pitch_range,
                animation: s.animation.clone(),
            });
            stances.push(leak(&s.animation));
        }
        let mut turret = None;
        let mut turret_mesh = None;
        let mounted = model
            .variant(variant)
            .and_then(|v| v.objects.iter().find(|o| !o.parent_marker.is_empty()))
            .cloned();
        if let Some(mount) = mounted {
            let attach = marker(&render, &bind, &mount.parent_marker).unwrap_or(Mat4::IDENTITY);
            let loaded = vehicle::read_vehicle(&mut self.set, mount.object).and_then(|t| {
                let m = vehicle::read_model(&mut self.set, t.model)?;
                let r = model::read_render_model(&mut self.set, m.render_model)?;
                Ok((t, r))
            });
            if let Ok((ttag, tm)) = loaded {
                let tbind = bind_pose(&tm);
                let gun = ttag
                    .weapons
                    .first()
                    .and_then(|&w| self.vehicle_weapon(w, weapons, meshes));
                let mesh = self.model_mesh(&tm);
                let part = VehiclePart::new(&tm, &mesh);
                let pivot = part
                    .yaw_node
                    .and_then(|n| tbind.get(n))
                    .map(|m| attach.transform_point3(m.w_axis.truncate()))
                    .unwrap_or(attach.w_axis.truncate());
                for s in player_seats(&ttag) {
                    let Some(m) = marker(&tm, &tbind, &s.marker) else {
                        continue;
                    };
                    let position = attach.transform_point3(m.w_axis.truncate());
                    let place = |n: &str| {
                        marker(&tm, &tbind, n).map(|m| attach.transform_point3(m.w_axis.truncate()))
                    };
                    seats.push(SeatDef {
                        role: SeatRole::Gunner,
                        position,
                        entry: place(&s.entry_marker).unwrap_or(position),
                        entry_radius: s.entry_radius.max(1.0),
                        eye: place(&s.camera_marker).unwrap_or(position + Vec3::Z * 0.55),
                        exposed: !s.has(seat_flags::INVISIBLE),
                        third_person: s.has(seat_flags::THIRD_PERSON_CAMERA),
                        weapon: gun,
                        pivot: Some(pivot),
                        pitch_range: s.pitch_range,
                        animation: s.animation.clone(),
                    });
                    stances.push(leak(&s.animation));
                }
                turret = Some((part, attach));
                turret_mesh = Some(mesh);
            }
        }

        let max_turn = match drive {
            Drive::Wheels => tag.max_left_turn.abs().to_radians().clamp(0.2, 1.0),
            _ => (tag.max_left_turn.abs().to_radians() * 2.5).max(2.0),
        };
        let def = VehicleDef {
            name: name.clone(),
            drive,
            mass: physics.as_ref().map_or(1000.0, |p| p.mass).max(50.0),
            health: if model.max_vitality > 0.0 {
                model.max_vitality
            } else {
                200.0
            },
            seats,
            wheels,
            pads,
            max_forward_speed: tag.max_forward_speed,
            max_reverse_speed: tag.max_reverse_speed,
            acceleration: tag.speed_acceleration,
            deceleration: tag.speed_deceleration.max(1.0),
            max_turn,
            max_slide: tag.max_left_slide.abs(),
            gravity_scale: if tag.gravity_scale > 0.0 {
                tag.gravity_scale
            } else {
                1.0
            },
            ..VehicleDef::default()
        }
        .with_hull(hull);
        let wheel_nodes = tag
            .friction
            .iter()
            .filter(|f| at(&f.marker).is_some())
            .map(|f| wheel_nodes(&render, f))
            .collect();
        let (engine, boost) = self.engine_sounds(&tag);
        let enter_sounds = stances.iter().map(|s| self.enter_sound(s)).collect();
        let assets = VehicleAssets {
            name: display_name(&name),
            enter_sounds,
            engine,
            boost,
            body,
            turret,
            wheel_nodes,
            steering_wheel: render.nodes.iter().position(|n| n.name == "steering_wheel"),
            seat_stances: stances,
        };
        Some((def, assets, body_mesh, turret_mesh))
    }

    /// The map's vehicles: each kind, and where each one appears (with
    /// its own copy of the meshes to pose).
    pub(super) fn vehicles(
        &mut self,
        weapons: &mut Vec<WeaponAssets>,
        meshes: &mut Vec<MeshData>,
    ) -> Vehicles {
        let mut out = Vehicles::default();
        let vehc = GroupTag::parse("vehc").expect("a group tag");
        let mut kinds: HashMap<(DatumIndex, String), Option<usize>> = HashMap::new();
        let mut templates: Vec<(MeshData, Option<MeshData>)> = Vec::new();
        for spawn in scenario::netgame_equipment(&mut self.set).unwrap_or_default() {
            let is_vehicle = self
                .set
                .locate(spawn.collection)
                .is_some_and(|(_, t)| t.group == vehc);
            if !is_vehicle {
                continue;
            }
            let Some((_, vehi, variant)) =
                vehicle::vehicle_collection(&mut self.set, spawn.collection)
                    .ok()
                    .and_then(|c| c.into_iter().next())
            else {
                continue;
            };
            let key = (vehi, variant.clone());
            let kind = match kinds.get(&key) {
                Some(&k) => k,
                None => {
                    let k = self.vehicle(vehi, &variant, weapons, meshes).map(
                        |(def, assets, mesh, turret)| {
                            out.defs.push(def);
                            out.kinds.push(assets);
                            templates.push((mesh, turret));
                            out.defs.len() - 1
                        },
                    );
                    kinds.insert(key, k);
                    k
                }
            };
            let Some(kind) = kind else { continue };
            let (mesh, turret) = &templates[kind];
            meshes.push(mesh.clone());
            let body = meshes.len() - 1;
            let turret = turret.as_ref().map(|t| {
                meshes.push(t.clone());
                meshes.len() - 1
            });
            out.meshes.push((body, turret));
            out.spawns.push(VehicleSpawn {
                def: kind,
                position: Vec3::from(spawn.position) + Vec3::Z * 0.05,
                yaw: spawn.rotation[0],
                respawn: match spawn.respawn_seconds {
                    0 => 30.0,
                    s => s as f32,
                },
            });
        }
        out
    }
}

/// Seat animation names live as long as the program (there are a few).
fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

/// The map's vehicles.
#[derive(Default)]
pub struct Vehicles {
    pub defs: Vec<VehicleDef>,
    pub kinds: Vec<VehicleAssets>,
    pub spawns: Vec<VehicleSpawn>,
    /// Per spawn: the meshes posed for that vehicle (and its turret).
    pub meshes: Vec<(usize, Option<usize>)>,
}
