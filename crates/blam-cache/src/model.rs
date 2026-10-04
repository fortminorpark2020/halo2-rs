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
const HLMT_ANIMATIONS: usize = 0x10;
const HLMT_VARIANTS: usize = 0x50;
const VARIANT_SIZE: usize = 0x40;
const VARIANT_REGIONS: usize = 0x14;
const VARIANT_REGION_SIZE: usize = 0x14;
const VARIANT_PERMUTATIONS: usize = 0x8;
const VARIANT_PERMUTATION_SIZE: usize = 0x20;
const OBJECT_CHANGE_COLORS: usize = 0xAC;
const CHANGE_COLOR_SIZE: usize = 0x10;
const INITIAL_PERMUTATION_SIZE: usize = 0x20;

/// The permutation a model variant shows in each region it names: `None`
/// hides the region. Regions it doesn't name show their first permutation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Variant {
    pub name: String,
    pub regions: Vec<(String, Option<String>)>,
}

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

/// A tag reference in an object's `hlmt` (render model, animations...).
fn object_model_ref(set: &mut MapSet, object: DatumIndex, field: usize) -> Result<DatumIndex> {
    let (_, _, obj) = set.tag_data(object)?;
    let hlmt = DatumIndex(u32_at(&obj, OBJECT_MODEL + 4));
    if hlmt == DatumIndex::NONE {
        return Err(Error::Corrupt("object has no model".into()));
    }
    let (_, _, model) = set.tag_data(hlmt)?;
    Ok(DatumIndex(u32_at(&model, field + 4)))
}

/// An object's look as the game shows it by default: its model's
/// "default" variant (or first), if the model has variants.
pub fn object_variant(set: &mut MapSet, object: DatumIndex) -> Result<Option<Variant>> {
    named_variant(set, object, "default")
}

/// The colours an object of model variant `variant` starts out in, one
/// per change colour (primary, secondary...): the middle of the range of
/// the variant's own initial permutation, else of one any variant may use,
/// else white (unchanged).
pub fn object_change_colors(
    set: &mut MapSet,
    object: DatumIndex,
    variant: &str,
) -> Result<Vec<[f32; 3]>> {
    let (src, _, obj) = set.tag_data(object)?;
    let file = set.get(src);
    let region = file.meta_region();
    let colors = file.read_block(region, &obj, OBJECT_CHANGE_COLORS, CHANGE_COLOR_SIZE)?;
    let mut out = Vec::new();
    for c in colors.as_chunks::<CHANGE_COLOR_SIZE>().0 {
        let perms = file.read_block(region, c, 0, INITIAL_PERMUTATION_SIZE)?;
        let perms = perms.as_chunks::<INITIAL_PERMUTATION_SIZE>().0;
        let called = |n: &str| perms.iter().find(|p| sid_name(file, &p[0x1C..]) == n);
        let rgb = |p: &[u8], o: usize| [0, 4, 8].map(|k| f32_at(p, o + k));
        out.push(match called(variant).or_else(|| called("")) {
            Some(p) => {
                let (lo, hi) = (rgb(p, 4), rgb(p, 0x10));
                [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0)
            }
            None => [1.0; 3],
        });
    }
    Ok(out)
}

/// An object's model variant called `name` (a vehicle collection's gauss
/// Warthog), else its default one (see `object_variant`).
pub fn named_variant(set: &mut MapSet, object: DatumIndex, name: &str) -> Result<Option<Variant>> {
    let (_, _, obj) = set.tag_data(object)?;
    let hlmt = DatumIndex(u32_at(&obj, OBJECT_MODEL + 4));
    if hlmt == DatumIndex::NONE {
        return Ok(None);
    }
    let (src, _, model) = set.tag_data(hlmt)?;
    let file = set.get(src);
    let region = file.meta_region();
    let variants = file.read_block(region, &model, HLMT_VARIANTS, VARIANT_SIZE)?;
    let variants = variants.as_chunks::<VARIANT_SIZE>().0;
    let called = |n: &str| variants.iter().find(|v| sid_name(file, &v[..]) == n);
    let Some(v) = called(name)
        .or_else(|| called("default"))
        .or(variants.first())
    else {
        return Ok(None);
    };
    let mut out = Variant {
        name: sid_name(file, v),
        regions: Vec::new(),
    };
    let regions = file.read_block(region, v, VARIANT_REGIONS, VARIANT_REGION_SIZE)?;
    for r in regions.as_chunks::<VARIANT_REGION_SIZE>().0 {
        let perms = file.read_block(region, r, VARIANT_PERMUTATIONS, VARIANT_PERMUTATION_SIZE)?;
        // A permutation the render model doesn't have (runtime index
        // 0xFF) hides the region.
        let shown = perms
            .as_chunks::<VARIANT_PERMUTATION_SIZE>()
            .0
            .first()
            .filter(|p| p[4] != 0xFF)
            .map(|p| sid_name(file, &p[..]));
        out.regions.push((sid_name(file, r), shown));
    }
    Ok(Some(out))
}

/// The string id a tag block element starts with, as text.
fn sid_name(file: &crate::mapset::Map, element: &[u8]) -> String {
    file.string_id(u32_at(element, 0)).unwrap_or("").to_string()
}

/// An object's render model as the game shows it by default (see
/// `object_variant`).
pub fn read_object_render_model(set: &mut MapSet, object: DatumIndex) -> Result<RenderModel> {
    let mode = object_render_model(set, object)?;
    let variant = object_variant(set, object).ok().flatten();
    read_render_model_variant(set, mode, variant.as_ref())
}

/// The render model of an object tag, via its `hlmt`.
pub fn object_render_model(set: &mut MapSet, object: DatumIndex) -> Result<DatumIndex> {
    let mode = object_model_ref(set, object, HLMT_RENDER_MODEL)?;
    if mode == DatumIndex::NONE {
        return Err(Error::Corrupt("model has no render model".into()));
    }
    Ok(mode)
}

/// The animation graph (`jmad`) of an object tag, via its `hlmt`.
pub fn object_animations(set: &mut MapSet, object: DatumIndex) -> Result<DatumIndex> {
    let jmad = object_model_ref(set, object, HLMT_ANIMATIONS)?;
    if jmad == DatumIndex::NONE {
        return Err(Error::Corrupt("model has no animations".into()));
    }
    Ok(jmad)
}

/// The render model of a `sky ` tag.
pub fn sky_render_model(set: &mut MapSet, sky: DatumIndex) -> Result<DatumIndex> {
    let (_, _, data) = set.tag_data(sky)?;
    let mode = DatumIndex(u32_at(&data, 4));
    if mode == DatumIndex::NONE {
        return Err(Error::Corrupt("sky has no render model".into()));
    }
    Ok(mode)
}

pub fn read_render_model(set: &mut MapSet, mode: DatumIndex) -> Result<RenderModel> {
    read_render_model_variant(set, mode, None)
}

/// A render model showing `variant`'s permutations (else each region's
/// first).
pub fn read_render_model_variant(
    set: &mut MapSet,
    mode: DatumIndex,
    variant: Option<&Variant>,
) -> Result<RenderModel> {
    let (src, _, data) = set.tag_data(mode)?;
    let file = set.get(src);
    let region = file.meta_region();
    let bounds = file.read_block(region, &data, MODE_BOUNDS, BOUNDS_SIZE)?;
    let regions = file.read_block(region, &data, MODE_REGIONS, REGION_SIZE)?;
    let sections = file.read_block(region, &data, MODE_SECTIONS, SECTION_SIZE)?;
    let nodes_raw = file.read_block(region, &data, MODE_NODES, NODE_SIZE)?;
    let groups = file.read_block(region, &data, MODE_MARKER_GROUPS, MARKER_GROUP_SIZE)?;
    let materials = file.read_block(region, &data, MODE_MATERIALS, MATERIAL_SIZE)?;

    // One section per region: its chosen permutation at the best detail
    // level.
    let mut wanted = Vec::new();
    for r in regions.as_chunks::<REGION_SIZE>().0 {
        let perms = file.read_block(region, r, 0x8, PERMUTATION_SIZE)?;
        let perms = perms.as_chunks::<PERMUTATION_SIZE>().0;
        let choice = variant.and_then(|v| {
            let region = sid_name(file, r);
            v.regions.iter().find(|(n, _)| *n == region)
        });
        let chosen = match choice {
            Some((_, None)) => None,
            Some((_, Some(p))) => perms
                .iter()
                .find(|q| sid_name(file, &q[..]) == *p)
                .or(perms.first()),
            None => perms.first(),
        };
        if let Some(p) = chosen {
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

const CLWD_GRID: usize = 0x10;
const CLWD_VERTICES: usize = 0x4C;
const CLOTH_VERTEX_SIZE: usize = 0x14;

/// A cloth (`clwd`): a grid of points hanging from a marker, like the flag.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Cloth {
    /// Points across and down.
    pub grid: (usize, usize),
    /// Rest positions (row by row, from the attached edge) and texture
    /// coordinates.
    pub vertices: Vec<([f32; 3], [f32; 2])>,
}

pub fn read_cloth(set: &mut MapSet, clwd: DatumIndex) -> Result<Cloth> {
    let (src, tag, d) = set.tag_data(clwd)?;
    if d.len() < CLWD_VERTICES + 8 {
        return Err(Error::Corrupt(format!("cloth tag {} too short", tag.name)));
    }
    let file = set.get(src);
    let region = file.meta_region();
    let raw = file.read_block(region, &d, CLWD_VERTICES, CLOTH_VERTEX_SIZE)?;
    let grid = (
        i16_at(&d, CLWD_GRID).max(0) as usize,
        i16_at(&d, CLWD_GRID + 2).max(0) as usize,
    );
    let vertices: Vec<_> = raw
        .as_chunks::<CLOTH_VERTEX_SIZE>()
        .0
        .iter()
        .map(|v| {
            (
                [f32_at(v, 0), f32_at(v, 4), f32_at(v, 8)],
                [f32_at(v, 0xC), f32_at(v, 0x10)],
            )
        })
        .collect();
    if grid.0 * grid.1 != vertices.len() || grid.0 < 2 || grid.1 < 2 {
        return Err(Error::Corrupt(format!(
            "cloth {} has {} points for a {}x{} grid",
            tag.name,
            vertices.len(),
            grid.0,
            grid.1
        )));
    }
    Ok(Cloth { grid, vertices })
}
