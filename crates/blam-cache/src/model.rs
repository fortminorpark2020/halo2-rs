//! Render models (`mode`) and the object tags that point at them.
//!
//! An object (weapon, vehicle, scenery...) names a model (`hlmt`), which names
//! the render model holding the actual triangles, a node skeleton and named
//! markers such as a weapon's muzzle.

use crate::mapset::MapSet;
use crate::render::{read_section, Section, SectionLayout};
use crate::{f32_at, i16_at, u32_at, DatumIndex, Error, Result};

const MODE_BOUNDS: usize = 0x14;
const BOUNDS_SIZE: usize = 0x38;
const MODE_REGIONS: usize = 0x1C;
const REGION_SIZE: usize = 0x10;
const PERMUTATION_SIZE: usize = 0x10;
const MODE_SECTIONS: usize = 0x24;
const SECTION_SIZE: usize = 0x5C;
const MODE_NODES: usize = 0x48;
const NODE_SIZE: usize = 0x60;
const MODE_MARKER_GROUPS: usize = 0x58;
const MARKER_GROUP_SIZE: usize = 0xC;
const MARKER_SIZE: usize = 0x24;
const MODE_MATERIALS: usize = 0x60;
const MATERIAL_SIZE: usize = 0x20;

/// Object tags (`weap`, `vehi`, `scen`, ...) start with the shared object
/// fields; the model reference sits at the same place in all of them.
const OBJECT_MODEL: usize = 0x34;
const HLMT_RENDER_MODEL: usize = 0x0;

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub name: String,
    pub parent: i16,
    /// Default pose relative to the parent.
    pub translation: [f32; 3],
    /// Quaternion (i, j, k, w).
    pub rotation: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Marker {
    pub node: u8,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub struct MarkerGroup {
    pub name: String,
    pub markers: Vec<Marker>,
}

#[derive(Debug, Clone, Default)]
pub struct RenderModel {
    /// The highest detail section of each region's first permutation.
    pub sections: Vec<Section>,
    pub shaders: Vec<DatumIndex>,
    pub nodes: Vec<Node>,
    pub markers: Vec<MarkerGroup>,
}

impl RenderModel {
    pub fn marker(&self, name: &str) -> Option<&Marker> {
        self.markers
            .iter()
            .find(|g| g.name == name)
            .and_then(|g| g.markers.first())
    }

    pub fn triangle_count(&self) -> usize {
        self.sections.iter().map(Section::triangle_count).sum()
    }
}

/// Positions and texture coordinates may be stored normalised to the model's
/// bounding box. PC caches store them in -1..1 around the box centre.
#[derive(Debug, Clone, Copy)]
struct Bounds {
    lo: [f32; 5],
    hi: [f32; 5],
}

impl Bounds {
    fn read(b: &[u8]) -> Bounds {
        let mut lo = [0f32; 5];
        let mut hi = [0f32; 5];
        for k in 0..5 {
            lo[k] = f32_at(b, k * 8);
            hi[k] = f32_at(b, k * 8 + 4);
        }
        Bounds { lo, hi }
    }

    fn expand(&self, k: usize, v: f32) -> f32 {
        let mid = (self.lo[k] + self.hi[k]) * 0.5;
        let half = (self.hi[k] - self.lo[k]) * 0.5;
        if half <= 0.0 {
            v
        } else {
            mid + v * half
        }
    }
}

/// The render model of an object tag, via its `hlmt`.
pub fn object_render_model(set: &mut MapSet, object: DatumIndex) -> Result<DatumIndex> {
    let (_, _, obj) = set.tag_data(object)?;
    let hlmt = DatumIndex(u32_at(&obj, OBJECT_MODEL + 4));
    if hlmt == DatumIndex::NONE {
        return Err(Error::Corrupt("object has no model".into()));
    }
    let (_, _, model) = set.tag_data(hlmt)?;
    let mode = DatumIndex(u32_at(&model, HLMT_RENDER_MODEL + 4));
    if mode == DatumIndex::NONE {
        return Err(Error::Corrupt("model has no render model".into()));
    }
    Ok(mode)
}

pub fn read_render_model(set: &mut MapSet, mode: DatumIndex) -> Result<RenderModel> {
    let (src, _, data) = set.tag_data(mode)?;
    let file = set.get(src);
    let region = file.meta_region();
    let bounds = file.read_block(region, &data, MODE_BOUNDS, BOUNDS_SIZE)?;
    let regions = file.read_block(region, &data, MODE_REGIONS, REGION_SIZE)?;
    let sections = file.read_block(region, &data, MODE_SECTIONS, SECTION_SIZE)?;
    let nodes_raw = file.read_block(region, &data, MODE_NODES, NODE_SIZE)?;
    let groups = file.read_block(region, &data, MODE_MARKER_GROUPS, MARKER_GROUP_SIZE)?;
    let materials = file.read_block(region, &data, MODE_MATERIALS, MATERIAL_SIZE)?;

    // One section per region: its first permutation at the best detail level.
    let mut wanted = Vec::new();
    for r in regions.as_chunks::<REGION_SIZE>().0 {
        let perms = file.read_block(region, r, 0x8, PERMUTATION_SIZE)?;
        if let Some(p) = perms.as_chunks::<PERMUTATION_SIZE>().0.first() {
            // L6 (hollywood) down to L1.
            let best = (0..6).rev().map(|l| i16_at(p, 4 + l * 2)).find(|&s| s >= 0);
            if let Some(s) = best {
                if !wanted.contains(&(s as usize)) {
                    wanted.push(s as usize);
                }
            }
        }
    }
    if wanted.is_empty() {
        wanted = (0..sections.len() / SECTION_SIZE).collect();
    }

    let mut markers = Vec::new();
    for g in groups.as_chunks::<MARKER_GROUP_SIZE>().0 {
        let list = file.read_block(region, g, 0x4, MARKER_SIZE)?;
        let name = file.string_id(u32_at(g, 0)).unwrap_or_default().to_string();
        let markers_in = list
            .as_chunks::<MARKER_SIZE>()
            .0
            .iter()
            .map(|m| Marker {
                node: m[2],
                translation: [f32_at(m, 4), f32_at(m, 8), f32_at(m, 12)],
                rotation: [f32_at(m, 16), f32_at(m, 20), f32_at(m, 24), f32_at(m, 28)],
            })
            .collect();
        markers.push(MarkerGroup {
            name,
            markers: markers_in,
        });
    }

    let nodes = nodes_raw
        .as_chunks::<NODE_SIZE>()
        .0
        .iter()
        .map(|n| Node {
            name: file.string_id(u32_at(n, 0)).unwrap_or("").to_string(),
            parent: i16_at(n, 4),
            translation: [f32_at(n, 0xC), f32_at(n, 0x10), f32_at(n, 0x14)],
            rotation: [
                f32_at(n, 0x18),
                f32_at(n, 0x1C),
                f32_at(n, 0x20),
                f32_at(n, 0x24),
            ],
        })
        .collect();

    let shaders = materials
        .as_chunks::<MATERIAL_SIZE>()
        .0
        .iter()
        .map(|m| {
            let s = DatumIndex(u32_at(m, 0xC));
            if s == DatumIndex::NONE {
                DatumIndex(u32_at(m, 0x4))
            } else {
                s
            }
        })
        .collect();

    let bounds = bounds
        .as_chunks::<BOUNDS_SIZE>()
        .0
        .first()
        .map(|b| Bounds::read(b));
    let all: Vec<&[u8; SECTION_SIZE]> = sections.as_chunks::<SECTION_SIZE>().0.iter().collect();
    let mut out = Vec::new();
    for i in wanted {
        let Some(header) = all.get(i) else { continue };
        let compressed = u16::from_le_bytes([header[0x1A], header[0x1B]]);
        let Some(mut s) = read_section(set, src, region, SectionLayout::MODEL, &header[..])? else {
            continue;
        };
        if let Some(b) = bounds {
            if compressed & 1 != 0 {
                for p in &mut s.positions {
                    for (k, v) in p.iter_mut().enumerate() {
                        *v = b.expand(k, *v);
                    }
                }
            }
            if compressed & 2 != 0 {
                for t in &mut s.uvs {
                    t[0] = b.expand(3, t[0]);
                    t[1] = b.expand(4, t[1]);
                }
            }
        }
        out.push(s);
    }

    Ok(RenderModel {
        sections: out,
        shaders,
        nodes,
        markers,
    })
}
