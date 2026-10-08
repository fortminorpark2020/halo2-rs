//! Effects (`effe`), contrails (`cont`) and lights (`ligh`): where an
//! effect plays on its object, the light it gives off, and the trails
//! rounds leave behind them.

use crate::mapset::MapSet;
use crate::{f32_at, u32_at, DatumIndex, GroupTag, Result};

/// `effe`: the markers its parts play at, and its events.
const EFFECT_LOCATIONS: usize = 0xC;
const LOCATION_SIZE: usize = 0x4;
const EFFECT_EVENTS: usize = 0x14;
const EVENT_SIZE: usize = 0x38;
const EVENT_PARTS: usize = 0x18;
const PART_SIZE: usize = 0x38;
const PART_TYPE: usize = 0xC;
/// `ligh`: the colour of its light (upper bound), and how long it shines.
const LIGHT_COLOR: usize = 0x64;
const LIGHT_DURATION: usize = 0xB0;
/// `cont`: points made per second, and the states each point goes through.
const CONTRAIL_RATE: usize = 0x4;
const CONTRAIL_POINT_STATES: usize = 0xE8;
const POINT_STATE_SIZE: usize = 0x40;

/// The markers an effect's parts play at, in its own order (a firing
/// effect's `primary_trigger`, `muzzle_flash`...).
pub fn effect_locations(set: &mut MapSet, effect: DatumIndex) -> Result<Vec<String>> {
    let Some((src, tag)) = set.locate(effect) else {
        return Ok(Vec::new());
    };
    if Some(tag.group) != GroupTag::parse("effe") {
        return Ok(Vec::new());
    }
    let (_, _, d) = set.tag_data(effect)?;
    let file = set.get(src);
    let region = file.meta_region();
    Ok(file
        .read_block(region, &d, EFFECT_LOCATIONS, LOCATION_SIZE)?
        .as_chunks::<LOCATION_SIZE>()
        .0
        .iter()
        .map(|l| file.string_id(u32_at(l, 0)).unwrap_or("").to_string())
        .collect())
}

/// A light an effect gives off: its colour (linear 0..1, upper bound) and
/// seconds it shines (0: as long as the effect).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectLight {
    pub color: [f32; 3],
    pub duration: f32,
}

/// The first light among an effect's parts, if it has one.
pub fn effect_light(set: &mut MapSet, effect: DatumIndex) -> Result<Option<EffectLight>> {
    let (Some(effe), Some(ligh)) = (GroupTag::parse("effe"), GroupTag::parse("ligh")) else {
        return Ok(None);
    };
    let Some((src, tag)) = set.locate(effect) else {
        return Ok(None);
    };
    if tag.group != effe {
        return Ok(None);
    }
    let (_, _, d) = set.tag_data(effect)?;
    let file = set.get(src);
    let region = file.meta_region();
    let mut parts = Vec::new();
    for e in file
        .read_block(region, &d, EFFECT_EVENTS, EVENT_SIZE)?
        .as_chunks::<EVENT_SIZE>()
        .0
    {
        parts.extend(file.read_block(region, e, EVENT_PARTS, PART_SIZE)?);
    }
    for p in parts.as_chunks::<PART_SIZE>().0 {
        let part = DatumIndex(u32_at(p, PART_TYPE + 4));
        if set.locate(part).is_some_and(|(_, t)| t.group == ligh) {
            let (_, _, l) = set.tag_data(part)?;
            if l.len() >= LIGHT_DURATION + 4 {
                return Ok(Some(EffectLight {
                    color: [
                        f32_at(&l, LIGHT_COLOR),
                        f32_at(&l, LIGHT_COLOR + 4),
                        f32_at(&l, LIGHT_COLOR + 8),
                    ],
                    duration: f32_at(&l, LIGHT_DURATION),
                }));
            }
        }
    }
    Ok(None)
}

/// One state a contrail's points pass through: how long they stay in it,
/// how long they take to become the next, and how they look meanwhile.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PointState {
    /// Seconds (shortest, longest).
    pub duration: (f32, f32),
    pub transition: (f32, f32),
    /// World units across.
    pub width: f32,
    /// Colour as red, green, blue and alpha (the tag stores alpha first),
    /// each point somewhere between these.
    pub color: ([f32; 4], [f32; 4]),
}

/// A contrail (`cont`): the trail a round leaves, as points that fade
/// through their states (Halo 2's tracers are contrails).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Contrail {
    /// Points made per second.
    pub rate: f32,
    pub states: Vec<PointState>,
}

impl Contrail {
    /// The colour of its points when new (the middle of the first state's
    /// range).
    pub fn color(&self) -> Option<[f32; 4]> {
        let s = self.states.first()?;
        Some(std::array::from_fn(|i| (s.color.0[i] + s.color.1[i]) * 0.5))
    }
}

/// A `colorf` stored alpha, red, green, blue: as red, green, blue, alpha.
fn argb(b: &[u8], at: usize) -> [f32; 4] {
    [
        f32_at(b, at + 4),
        f32_at(b, at + 8),
        f32_at(b, at + 12),
        f32_at(b, at),
    ]
}

pub fn read_contrail(set: &mut MapSet, cont: DatumIndex) -> Result<Contrail> {
    let (src, _, d) = set.tag_data(cont)?;
    if d.len() < CONTRAIL_POINT_STATES + 8 {
        return Ok(Contrail::default());
    }
    let file = set.get(src);
    let region = file.meta_region();
    let states = file
        .read_block(region, &d, CONTRAIL_POINT_STATES, POINT_STATE_SIZE)?
        .as_chunks::<POINT_STATE_SIZE>()
        .0
        .iter()
        .map(|s| PointState {
            duration: (f32_at(s, 0x0), f32_at(s, 0x4)),
            transition: (f32_at(s, 0x8), f32_at(s, 0xC)),
            width: f32_at(s, 0x18),
            color: (argb(s, 0x1C), argb(s, 0x2C)),
        })
        .collect();
    Ok(Contrail {
        rate: f32_at(&d, CONTRAIL_RATE),
        states,
    })
}

/// The contrails attached to an object (a projectile's tracer, or the two
/// ribbons of the Beam Rifle's bolt).
pub fn object_contrails(set: &mut MapSet, object: DatumIndex) -> Result<Vec<Contrail>> {
    let Some(cont) = GroupTag::parse("cont") else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (d, _) in crate::vehicle::object_attachments(set, object)? {
        if set.locate(d).is_some_and(|(_, t)| t.group == cont) {
            let c = read_contrail(set, d)?;
            if !c.states.is_empty() {
                out.push(c);
            }
        }
    }
    Ok(out)
}
