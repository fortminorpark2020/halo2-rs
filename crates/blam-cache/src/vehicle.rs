//! Vehicles (`vehi`): their seats, weapons and driving tuning, the model
//! (`hlmt`) variants that attach turrets, and the physics model's (`phmo`)
//! mass and hull shapes.

use crate::mapset::{Map, MapSet};
use crate::{f32_at, i16_at, i32_at, u32_at, DatumIndex, Error, Result};

const OBJECT_BOUNDING_RADIUS: usize = 0x4;
const OBJECT_DEFAULT_VARIANT: usize = 0x30;
const OBJECT_MODEL: usize = 0x34;
const OBJECT_ATTACHMENTS: usize = 0x94;
const ATTACHMENT_SIZE: usize = 0x18;
const UNIT_WEAPONS: usize = 0x1C0;
const UNIT_SEATS: usize = 0x1C8;
const SEAT_SIZE: usize = 0xB0;
const SEAT_CAMERA_TRACKS: usize = 0x74;
const VEHI_TYPE: usize = 0x1F0;
const VEHI_ANTI_GRAVITY: usize = 0x2E8;
const ANTI_GRAVITY_SIZE: usize = 0x4C;
const VEHI_FRICTION: usize = 0x2F0;
const FRICTION_SIZE: usize = 0x4C;

const HLMT_RENDER_MODEL: usize = 0x0;
const HLMT_ANIMATIONS: usize = 0x10;
const HLMT_PHYSICS_MODEL: usize = 0x20;
const HLMT_VARIANTS: usize = 0x50;
const VARIANT_SIZE: usize = 0x38;
const VARIANT_OBJECTS: usize = 0x1C;
const VARIANT_OBJECT_SIZE: usize = 0x10;
const HLMT_DAMAGE_INFO: usize = 0x60;
const DAMAGE_INFO_SIZE: usize = 0xF8;

const TRAK_POINTS: usize = 0x4;
const TRAK_POINT_SIZE: usize = 0x1C;

const PHMO_MASS: usize = 0x4;
const PHMO_BOXES: usize = 0x60;
const BOX_SIZE: usize = 0x90;
const PHMO_POLYHEDRA: usize = 0x70;
const POLYHEDRON_SIZE: usize = 0x100;
const PHMO_PILLS: usize = 0x58;
const PILL_SIZE: usize = 0x50;
const PHMO_SPHERES: usize = 0x48;
const SPHERE_SIZE: usize = 0x80;
const PHMO_RIGID_BODIES: usize = 0x38;
const RIGID_BODY_SIZE: usize = 0x90;
const PHMO_LISTS: usize = 0x90;
const LIST_SIZE: usize = 0x38;
const SHAPE_SPHERE: i16 = 0;
const SHAPE_PILL: i16 = 1;
const SHAPE_BOX: i16 = 2;
const SHAPE_POLYHEDRON: i16 = 4;
const SHAPE_LIST: i16 = 14;

const VEHICLE_COLLECTION_ENTRY_SIZE: usize = 0x10;

/// Seat flag bits.
pub mod seat_flags {
    /// Completely enclosed by the vehicle.
    pub const INVISIBLE: u32 = 1 << 0;
    pub const DRIVER: u32 = 1 << 2;
    pub const GUNNER: u32 = 1 << 3;
    pub const THIRD_PERSON_CAMERA: u32 = 1 << 4;
    pub const ALLOWS_WEAPONS: u32 = 1 << 5;
    pub const BOARDING: u32 = 1 << 11;
    pub const INVALID_FOR_PLAYER: u32 = 1 << 16;
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Seat {
    pub flags: u32,
    /// The rider's animation mode in this seat ("warthog_d", "ghost_d"...).
    pub animation: String,
    pub marker: String,
    pub entry_marker: String,
    pub camera_marker: String,
    /// The camera track (`trak`) the rider's view follows, if any.
    pub camera_track: DatumIndex,
    /// Radians either way the rider may look, around and up/down.
    pub yaw_range: [f32; 2],
    pub pitch_range: [f32; 2],
    pub built_in_gunner: DatumIndex,
    pub entry_radius: f32,
    /// How fast the rider's controller look turns in this seat, degrees a
    /// second, standing still and at speed: across (+0x44, +0x48) and up
    /// or down (+0x4C, +0x50). Zero keeps the profile's rate.
    pub yaw_rate: [f32; 2],
    pub pitch_rate: [f32; 2],
    /// The vehicle speeds those run between (world units a second,
    /// +0x54/+0x58), and the curve between (+0x5C, an exponent; zero
    /// is a straight line).
    pub speed_range: [f32; 2],
    pub speed_exponent: f32,
}

impl Seat {
    pub fn has(&self, flag: u32) -> bool {
        self.flags & flag != 0
    }
}

/// What kind of vehicle the game treats it as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VehicleType {
    HumanTank,
    #[default]
    HumanJeep,
    HumanBoat,
    HumanPlane,
    AlienScout,
    AlienFighter,
    Turret,
}

/// A point the vehicle hovers on (Ghost, Banshee).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AntiGravityPoint {
    pub marker: String,
    pub strength: f32,
    pub offset: f32,
    pub height: f32,
    pub damp: f32,
    pub radius: f32,
}

/// A wheel (or other point touching the ground).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FrictionPoint {
    pub marker: String,
    pub flags: u32,
    pub mass_fraction: f32,
    pub radius: f32,
}

impl FrictionPoint {
    /// Wheel flags: driven by the engine, and turned by steering.
    pub const POWERED: u32 = 1 << 1;
    pub const FRONT_TURNING: u32 = 1 << 2;
    pub const REAR_TURNING: u32 = 1 << 3;
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct VehicleTag {
    /// The vehicle's model (`hlmt`).
    pub model: DatumIndex,
    pub default_variant: String,
    pub bounding_radius: f32,
    /// Weapons built into the vehicle (the Ghost's guns...).
    pub weapons: Vec<DatumIndex>,
    pub seats: Vec<Seat>,
    pub kind: VehicleType,
    pub max_forward_speed: f32,
    pub max_reverse_speed: f32,
    pub speed_acceleration: f32,
    pub speed_deceleration: f32,
    pub max_left_turn: f32,
    pub max_right_turn: f32,
    pub wheel_circumference: f32,
    pub turn_rate: f32,
    pub max_left_slide: f32,
    pub max_right_slide: f32,
    pub slide_acceleration: f32,
    pub slide_deceleration: f32,
    pub flying_torque_scale: f32,
    pub air_friction_deceleration: f32,
    pub thrust_scale: f32,
    pub ground_friction: f32,
    pub ground_depth: f32,
    pub ground_damp_factor: f32,
    pub gravity_scale: f32,
    pub radius: f32,
    pub anti_gravity: Vec<AntiGravityPoint>,
    pub friction: Vec<FrictionPoint>,
    /// What the object carries at its markers: lights, effects and looping
    /// sounds (the engine).
    pub attachments: Vec<(DatumIndex, String)>,
}

/// A child object a model variant attaches (the Warthog's turret).
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    pub parent_marker: String,
    pub child_marker: String,
    pub object: DatumIndex,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelVariant {
    pub name: String,
    pub objects: Vec<Attachment>,
}

/// An object's model (`hlmt`): what it is made of and how much it takes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModelTag {
    pub render_model: DatumIndex,
    pub animations: DatumIndex,
    pub physics_model: DatumIndex,
    pub variants: Vec<ModelVariant>,
    pub max_vitality: f32,
    pub max_shield: f32,
}

impl ModelTag {
    /// The variant called `name`, or the first.
    pub fn variant(&self, name: &str) -> Option<&ModelVariant> {
        self.variants
            .iter()
            .find(|v| v.name == name)
            .or(self.variants.first())
    }
}

/// A box in the physics model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HullBox {
    pub center: [f32; 3],
    pub half_extents: [f32; 3],
    /// Rows of the box's rotation (its axes in its node's space).
    pub axes: [[f32; 3]; 3],
    /// The render model node it moves with; it is placed in that node's
    /// space.
    pub node: i16,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PhysicsModel {
    pub mass: f32,
    /// The hull's shapes as boxes: boxes, the bounds of convex pieces, and
    /// the bounds of pills and spheres.
    pub boxes: Vec<HullBox>,
}

fn v3(b: &[u8], at: usize) -> [f32; 3] {
    [f32_at(b, at), f32_at(b, at + 4), f32_at(b, at + 8)]
}

fn sid(file: &Map, b: &[u8], at: usize) -> String {
    file.string_id(u32_at(b, at))
        .unwrap_or_default()
        .to_string()
}

fn datum(b: &[u8], tag_ref: usize) -> DatumIndex {
    DatumIndex(u32_at(b, tag_ref + 4))
}

/// What any object tag (a vehicle, a projectile...) carries attached:
/// lights, effects and looping sounds, each with the marker it sits at.
pub fn object_attachments(
    set: &mut MapSet,
    object: DatumIndex,
) -> Result<Vec<(DatumIndex, String)>> {
    let (src, _, data) = set.tag_data(object)?;
    if data.len() < OBJECT_ATTACHMENTS + 8 {
        return Err(Error::Corrupt("object tag too short".into()));
    }
    let file = set.get(src);
    let region = file.meta_region();
    Ok(file
        .read_block(region, &data, OBJECT_ATTACHMENTS, ATTACHMENT_SIZE)?
        .as_chunks::<ATTACHMENT_SIZE>()
        .0
        .iter()
        .map(|a| (datum(a, 0), sid(file, a, 0x8)))
        .filter(|a| a.0 != DatumIndex::NONE)
        .collect())
}

/// Read a vehicle tag.
/// A point on a camera track (`trak`): where the camera sits, from the
/// camera marker (x along the look, z up), while it looks along
/// `orientation`. A track's points run from looking straight down to
/// straight up.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CameraPoint {
    pub position: [f32; 3],
    /// i, j, k, w.
    pub orientation: [f32; 4],
}

pub fn read_camera_track(set: &mut MapSet, trak: DatumIndex) -> Result<Vec<CameraPoint>> {
    let (src, tag, data) = set.tag_data(trak)?;
    if tag.group.to_string() != "trak" {
        return Err(Error::Corrupt(format!(
            "{} is not a camera track",
            tag.name
        )));
    }
    let file = set.get(src);
    let region = file.meta_region();
    Ok(file
        .read_block(region, &data, TRAK_POINTS, TRAK_POINT_SIZE)?
        .as_chunks::<TRAK_POINT_SIZE>()
        .0
        .iter()
        .map(|p| CameraPoint {
            position: [f32_at(p, 0), f32_at(p, 4), f32_at(p, 8)],
            orientation: [f32_at(p, 12), f32_at(p, 16), f32_at(p, 20), f32_at(p, 24)],
        })
        .collect())
}

pub fn read_vehicle(set: &mut MapSet, vehi: DatumIndex) -> Result<VehicleTag> {
    let (src, tag, data) = set.tag_data(vehi)?;
    if tag.group.to_string() != "vehi" {
        return Err(Error::Corrupt(format!("{} is not a vehicle", tag.name)));
    }
    if data.len() < VEHI_FRICTION + 8 {
        return Err(Error::Corrupt("vehicle tag too short".into()));
    }
    let file = set.get(src);
    let region = file.meta_region();
    let weapons = file
        .read_block(region, &data, UNIT_WEAPONS, 8)?
        .as_chunks::<8>()
        .0
        .iter()
        .map(|w| datum(w, 0))
        .filter(|d| *d != DatumIndex::NONE)
        .collect();
    let seat_data = file.read_block(region, &data, UNIT_SEATS, SEAT_SIZE)?;
    let mut seats = Vec::new();
    for s in seat_data.as_chunks::<SEAT_SIZE>().0 {
        let camera_track = file
            .read_block(region, s, SEAT_CAMERA_TRACKS, 8)?
            .as_chunks::<8>()
            .0
            .first()
            .map_or(DatumIndex::NONE, |t| datum(t, 0));
        seats.push(Seat {
            flags: u32_at(s, 0),
            animation: sid(file, s, 0x4),
            marker: sid(file, s, 0x8),
            entry_marker: sid(file, s, 0xC),
            camera_marker: sid(file, s, 0x60),
            camera_track,
            pitch_range: [f32_at(s, 0x6C), f32_at(s, 0x70)],
            yaw_range: [f32_at(s, 0x88), f32_at(s, 0x8C)],
            built_in_gunner: datum(s, 0x90),
            entry_radius: f32_at(s, 0x98),
            yaw_rate: [f32_at(s, 0x44), f32_at(s, 0x48)],
            pitch_rate: [f32_at(s, 0x4C), f32_at(s, 0x50)],
            speed_range: [f32_at(s, 0x54), f32_at(s, 0x58)],
            speed_exponent: f32_at(s, 0x5C),
        });
    }
    let anti_gravity = file
        .read_block(region, &data, VEHI_ANTI_GRAVITY, ANTI_GRAVITY_SIZE)?
        .as_chunks::<ANTI_GRAVITY_SIZE>()
        .0
        .iter()
        .map(|a| AntiGravityPoint {
            marker: sid(file, a, 0),
            strength: f32_at(a, 0x8),
            offset: f32_at(a, 0xC),
            height: f32_at(a, 0x10),
            damp: f32_at(a, 0x14),
            radius: f32_at(a, 0x20),
        })
        .collect();
    let friction = file
        .read_block(region, &data, VEHI_FRICTION, FRICTION_SIZE)?
        .as_chunks::<FRICTION_SIZE>()
        .0
        .iter()
        .map(|f| FrictionPoint {
            marker: sid(file, f, 0),
            flags: u32_at(f, 0x4),
            mass_fraction: f32_at(f, 0x8),
            radius: f32_at(f, 0xC),
        })
        .collect();
    let attachments = file
        .read_block(region, &data, OBJECT_ATTACHMENTS, ATTACHMENT_SIZE)?
        .as_chunks::<ATTACHMENT_SIZE>()
        .0
        .iter()
        .map(|a| (datum(a, 0), sid(file, a, 0x8)))
        .filter(|a| a.0 != DatumIndex::NONE)
        .collect();
    let kind = match i16_at(&data, VEHI_TYPE) {
        0 => VehicleType::HumanTank,
        2 => VehicleType::HumanBoat,
        3 => VehicleType::HumanPlane,
        4 => VehicleType::AlienScout,
        5 => VehicleType::AlienFighter,
        6 => VehicleType::Turret,
        _ => VehicleType::HumanJeep,
    };
    let f = |at: usize| f32_at(&data, at);
    Ok(VehicleTag {
        model: datum(&data, OBJECT_MODEL),
        default_variant: sid(file, &data, OBJECT_DEFAULT_VARIANT),
        bounding_radius: f(OBJECT_BOUNDING_RADIUS),
        weapons,
        seats,
        kind,
        max_forward_speed: f(0x1F4),
        max_reverse_speed: f(0x1F8),
        speed_acceleration: f(0x1FC),
        speed_deceleration: f(0x200),
        max_left_turn: f(0x204),
        max_right_turn: f(0x208),
        wheel_circumference: f(0x20C),
        turn_rate: f(0x210),
        max_left_slide: f(0x22C),
        max_right_slide: f(0x230),
        slide_acceleration: f(0x234),
        slide_deceleration: f(0x238),
        flying_torque_scale: f(0x270),
        air_friction_deceleration: f(0x27C),
        thrust_scale: f(0x280),
        ground_friction: f(0x2B0),
        ground_depth: f(0x2B4),
        ground_damp_factor: f(0x2B8),
        gravity_scale: f(0x2E0),
        radius: f(0x2E4),
        anti_gravity,
        friction,
        attachments,
    })
}

/// Read an object's model tag.
pub fn read_model(set: &mut MapSet, hlmt: DatumIndex) -> Result<ModelTag> {
    let (src, _, data) = set.tag_data(hlmt)?;
    let file = set.get(src);
    let region = file.meta_region();
    let mut variants = Vec::new();
    for v in file
        .read_block(region, &data, HLMT_VARIANTS, VARIANT_SIZE)?
        .as_chunks::<VARIANT_SIZE>()
        .0
    {
        let objects = file
            .read_block(region, v, VARIANT_OBJECTS, VARIANT_OBJECT_SIZE)?
            .as_chunks::<VARIANT_OBJECT_SIZE>()
            .0
            .iter()
            .map(|o| Attachment {
                parent_marker: sid(file, o, 0),
                child_marker: sid(file, o, 4),
                object: datum(o, 8),
            })
            .filter(|a| a.object != DatumIndex::NONE)
            .collect();
        variants.push(ModelVariant {
            name: sid(file, v, 0),
            objects,
        });
    }
    let damage = file.read_block(region, &data, HLMT_DAMAGE_INFO, DAMAGE_INFO_SIZE)?;
    let (max_vitality, max_shield) = match damage.as_chunks::<DAMAGE_INFO_SIZE>().0.first() {
        Some(d) => (f32_at(d, 0x28), f32_at(d, 0x8C)),
        None => (0.0, 0.0),
    };
    Ok(ModelTag {
        render_model: datum(&data, HLMT_RENDER_MODEL),
        animations: datum(&data, HLMT_ANIMATIONS),
        physics_model: datum(&data, HLMT_PHYSICS_MODEL),
        variants,
        max_vitality,
        max_shield,
    })
}

/// Read a physics model's mass and hull.
pub fn read_physics_model(set: &mut MapSet, phmo: DatumIndex) -> Result<PhysicsModel> {
    let (src, _, data) = set.tag_data(phmo)?;
    let file = set.get(src);
    let region = file.meta_region();
    // Every shape as a box, by (shape type, index).
    let mut shapes: Vec<((i16, i16), HullBox)> = Vec::new();
    let blocks: [(i16, usize, usize); 4] = [
        (SHAPE_SPHERE, PHMO_SPHERES, SPHERE_SIZE),
        (SHAPE_PILL, PHMO_PILLS, PILL_SIZE),
        (SHAPE_BOX, PHMO_BOXES, BOX_SIZE),
        (SHAPE_POLYHEDRON, PHMO_POLYHEDRA, POLYHEDRON_SIZE),
    ];
    for (kind, at, size) in blocks {
        let raw = file.read_block(region, &data, at, size)?;
        for (k, e) in raw.chunks_exact(size).enumerate() {
            let r = f32_at(e, 0x2C);
            let hull = match kind {
                SHAPE_SPHERE => HullBox {
                    center: v3(e, 0x70),
                    half_extents: [r; 3],
                    axes: IDENTITY,
                    node: 0,
                },
                SHAPE_PILL => {
                    let (a, b) = (v3(e, 0x30), v3(e, 0x40));
                    HullBox {
                        center: [0, 1, 2].map(|k| (a[k] + b[k]) * 0.5),
                        half_extents: [0, 1, 2].map(|k| (a[k] - b[k]).abs() * 0.5 + r),
                        axes: IDENTITY,
                        node: 0,
                    }
                }
                SHAPE_BOX => HullBox {
                    center: v3(e, 0x80),
                    half_extents: v3(e, 0x30).map(|h| h + r),
                    axes: [v3(e, 0x50), v3(e, 0x60), v3(e, 0x70)],
                    node: 0,
                },
                _ => HullBox {
                    center: v3(e, 0x40),
                    half_extents: v3(e, 0x30).map(|h| h + r),
                    axes: IDENTITY,
                    node: 0,
                },
            };
            shapes.push(((kind, k as i16), hull));
        }
    }
    // Which node each shape moves with, through the rigid bodies (whose
    // shape may be a list of shapes).
    let lists = file.read_block(region, &data, PHMO_LISTS, LIST_SIZE)?;
    let lists = lists.as_chunks::<LIST_SIZE>().0;
    let bodies = file.read_block(region, &data, PHMO_RIGID_BODIES, RIGID_BODY_SIZE)?;
    for b in bodies.as_chunks::<RIGID_BODY_SIZE>().0 {
        let node = i16_at(b, 0);
        let shape = (i16_at(b, 0x38), i16_at(b, 0x3A));
        let mut parts = vec![shape];
        if shape.0 == SHAPE_LIST {
            if let Some(l) = usize::try_from(shape.1).ok().and_then(|i| lists.get(i)) {
                let n = i32_at(l, 0x10).clamp(0, 4) as usize;
                parts = (0..n)
                    .map(|k| (i16_at(l, 0x18 + k * 8), i16_at(l, 0x1A + k * 8)))
                    .collect();
            }
        }
        for (key, hull) in &mut shapes {
            if parts.contains(key) {
                hull.node = node;
            }
        }
    }
    Ok(PhysicsModel {
        mass: f32_at(&data, PHMO_MASS),
        boxes: shapes.into_iter().map(|(_, h)| h).collect(),
    })
}

const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// A vehicle collection's vehicles: (weight, vehicle, model variant).
pub fn vehicle_collection(
    set: &mut MapSet,
    vehc: DatumIndex,
) -> Result<Vec<(f32, DatumIndex, String)>> {
    let (src, _, data) = set.tag_data(vehc)?;
    let file = set.get(src);
    let region = file.meta_region();
    let entries = file.read_block(region, &data, 0, VEHICLE_COLLECTION_ENTRY_SIZE)?;
    Ok(entries
        .as_chunks::<VEHICLE_COLLECTION_ENTRY_SIZE>()
        .0
        .iter()
        .map(|e| (f32_at(e, 0), datum(e, 4), sid(file, e, 0xC)))
        .filter(|(_, d, _)| *d != DatumIndex::NONE)
        .collect())
}
