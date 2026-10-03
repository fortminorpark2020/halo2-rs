//! Render geometry: the textured triangles the game actually draws.
//!
//! Render geometry lives in "sections" (BSP clusters, instanced geometry
//! definitions, model sections). Each section's vertices and indices are stored
//! in a raw resource block outside the tag data; the section's tag block lists
//! where each buffer sits inside that block.

use crate::mapset::{MapSet, Source};
use crate::{f32_at, i16_at, i32_at, u32_at, DatumIndex, Error, Region, Result, StructureBsp};

/// Where a section-like tag block element keeps its fields. BSP clusters and
/// instanced geometry share one layout; render model sections use another.
#[derive(Debug, Clone, Copy)]
pub struct SectionLayout {
    vertex_count: usize,
    face_count: usize,
    data_pointer: usize,
    data_size: usize,
    /// BSP: size of the section header. Models: size of the resource data.
    size_field: usize,
    resources: usize,
    model: bool,
}

impl SectionLayout {
    pub const BSP: SectionLayout = SectionLayout {
        vertex_count: 0x0,
        face_count: 0x2,
        data_pointer: 0x28,
        data_size: 0x2C,
        size_field: 0x30,
        resources: 0x38,
        model: false,
    };
    pub const MODEL: SectionLayout = SectionLayout {
        vertex_count: 0x4,
        face_count: 0x6,
        data_pointer: 0x38,
        data_size: 0x3C,
        size_field: 0x44,
        resources: 0x48,
        model: true,
    };

    /// Where the resource buffers start inside the section's data block.
    fn base(&self, data_size: i32, size_field: i32) -> Option<usize> {
        let base = if self.model {
            data_size - size_field - 4
        } else {
            size_field + 8
        };
        usize::try_from(base).ok()
    }
}

const RESOURCE_SIZE: usize = 0x10;

/// Halo 2 PC resource kinds (by the locator values the cache files use).
const RES_INDICES: i16 = 32;
const RES_VERTEX_BUFFERS: i16 = 56;
const RES_NODE_MAP: i16 = 100;
const SUBMESH_SIZE: usize = 72;

const SBSP_CLUSTERS: usize = 0x9C;
const CLUSTER_SIZE: usize = 0xB0;
const SBSP_MATERIALS: usize = 0xA4;
const MATERIAL_SIZE: usize = 0x20;
const SBSP_INSTANCE_DEFS: usize = 0x138;
const INSTANCE_DEF_SIZE: usize = 0xC8;
const SBSP_INSTANCES: usize = 0x140;
const INSTANCE_SIZE: usize = 0x58;

#[derive(Debug, Clone, Default)]
pub struct Part {
    /// Index into the owner's material (shader) list.
    pub material: i16,
    /// Triangle list indices into the section's vertices.
    pub indices: Vec<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct Section {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub parts: Vec<Part>,
    /// Model sections only (empty for level geometry): up to four nodes each
    /// vertex follows (already remapped through the section's node map) and
    /// how much it follows each, strongest first; weights add up to 1.
    pub bones: Vec<[u8; 4]>,
    pub weights: Vec<[f32; 4]>,
    /// Level geometry only: where each vertex falls on its lightmap page
    /// (empty when the section has no lightmap).
    pub lightmap_uvs: Vec<[f32; 2]>,
}

impl Section {
    pub fn triangle_count(&self) -> usize {
        self.parts.iter().map(|p| p.indices.len() / 3).sum()
    }

    /// Apply an instance placement (uniform scale, basis vectors, translation).
    pub fn transformed(
        &self,
        scale: f32,
        fwd: [f32; 3],
        left: [f32; 3],
        up: [f32; 3],
        pos: [f32; 3],
    ) -> Section {
        let rot = |v: [f32; 3]| -> [f32; 3] {
            let mut o = [0f32; 3];
            for k in 0..3 {
                o[k] = v[0] * fwd[k] + v[1] * left[k] + v[2] * up[k];
            }
            o
        };
        Section {
            positions: self
                .positions
                .iter()
                .map(|&p| {
                    let r = rot(p);
                    [
                        pos[0] + scale * r[0],
                        pos[1] + scale * r[1],
                        pos[2] + scale * r[2],
                    ]
                })
                .collect(),
            normals: self.normals.iter().map(|&n| rot(n)).collect(),
            uvs: self.uvs.clone(),
            parts: self.parts.clone(),
            bones: self.bones.clone(),
            weights: self.weights.clone(),
            lightmap_uvs: self.lightmap_uvs.clone(),
        }
    }
}

/// What a level section is: a BSP cluster or a placed instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionOwner {
    Cluster(usize),
    Instance(usize),
}

/// Level render geometry plus the shader each material slot uses.
#[derive(Debug, Clone, Default)]
pub struct LevelGeometry {
    pub sections: Vec<Section>,
    /// For each section, the cluster or instance it came from.
    pub owners: Vec<SectionOwner>,
    /// Vertex count of every instance (0 where its geometry is missing).
    pub instance_vertices: Vec<usize>,
    pub shaders: Vec<DatumIndex>,
}

/// Expand a triangle strip to a list, dropping degenerate triangles.
pub fn strip_to_list(strip: &[u16]) -> Vec<u32> {
    let mut out = Vec::with_capacity(strip.len().saturating_sub(2) * 3);
    for i in 0..strip.len().saturating_sub(2) {
        let (a, b, c) = (strip[i], strip[i + 1], strip[i + 2]);
        if a == b || b == c || a == c {
            continue;
        }
        if i % 2 == 0 {
            out.extend_from_slice(&[a as u32, b as u32, c as u32]);
        } else {
            out.extend_from_slice(&[a as u32, c as u32, b as u32]);
        }
    }
    out
}

struct Resource {
    kind: i16,
    sub: i16,
    size: usize,
    offset: usize,
}

fn slice(data: &[u8], start: usize, len: usize) -> Result<&[u8]> {
    data.get(start..start + len).ok_or_else(|| {
        Error::Corrupt(format!(
            "section buffer {start:#x}+{len:#x} past end {:#x}",
            data.len()
        ))
    })
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

/// Decode one section. `header` is the section's tag block element, read from
/// the file `owner` (whose meta `region` its pointers refer to).
pub fn read_section(
    set: &mut MapSet,
    owner: Source,
    region: Region,
    layout: SectionLayout,
    header: &[u8],
) -> Result<Option<Section>> {
    let vertex_count = u16_at(header, layout.vertex_count) as usize;
    let face_count = u16_at(header, layout.face_count) as usize;
    if vertex_count == 0 {
        return Ok(None);
    }
    let pointer = u32_at(header, layout.data_pointer);
    let data_size = i32_at(header, layout.data_size);
    let size_field = i32_at(header, layout.size_field);
    let Some(base) = layout.base(data_size, size_field) else {
        return Ok(None);
    };
    if data_size <= 0 || size_field < 0 {
        return Ok(None);
    }
    let res_raw = set
        .get(owner)
        .read_block(region, header, layout.resources, RESOURCE_SIZE)?;
    let resources: Vec<Resource> = res_raw
        .as_chunks::<RESOURCE_SIZE>()
        .0
        .iter()
        .map(|r| Resource {
            kind: i16_at(r, 4),
            sub: i16_at(r, 6),
            size: i32_at(r, 8).max(0) as usize,
            offset: i32_at(r, 12).max(0) as usize,
        })
        .collect();
    let data = set.read_resource(owner, pointer, data_size as usize)?;

    if data.len() < 42 {
        return Ok(None);
    }
    let index_count = u16_at(&data, 40) as usize;
    let find = |kind: i16, sub: Option<i16>| {
        resources
            .iter()
            .find(|r| r.kind == kind && sub.is_none_or(|s| r.sub == s))
    };
    let (Some(submeshes), Some(indices), Some(verts)) = (
        resources.first(),
        find(RES_INDICES, None),
        find(RES_VERTEX_BUFFERS, Some(0)),
    ) else {
        return Ok(None);
    };

    let raw_idx = slice(&data, base + indices.offset, index_count * 2)?;
    let idx: Vec<u16> = raw_idx
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes(*b))
        .collect();
    let mut is_list = face_count * 3 == index_count;
    // A strip can coincidentally have face_count * 3 indices; real lists have no repeats per triangle.
    if is_list
        && idx
            .as_chunks::<3>()
            .0
            .iter()
            .take(10)
            .any(|t| t[0] == t[1] || t[1] == t[2] || t[0] == t[2])
    {
        is_list = false;
    }

    let stride = verts.size / vertex_count;
    if stride < 12 {
        return Err(Error::Corrupt(format!("vertex stride {stride} too small")));
    }
    let vbuf = slice(&data, base + verts.offset, stride * vertex_count)?;
    let positions = (0..vertex_count)
        .map(|i| {
            let o = i * stride;
            [f32_at(vbuf, o), f32_at(vbuf, o + 4), f32_at(vbuf, o + 8)]
        })
        .collect();
    // Model vertices carry their nodes right after the position: rigid-boned
    // vertices (16 bytes) one node index, skinned ones (20 bytes) four node
    // indices then four byte weights. Rigid sections follow one node.
    let (mut bones, mut weights) = (Vec::new(), Vec::new());
    if layout.model {
        let map = match find(RES_NODE_MAP, None) {
            Some(r) => slice(&data, base + r.offset, r.size)?.to_vec(),
            None => Vec::new(),
        };
        let remap = |n: u8| map.get(n as usize).copied().unwrap_or(n);
        for i in 0..vertex_count {
            let v = &vbuf[i * stride..(i + 1) * stride];
            if stride >= 20 {
                let mut w = [0, 1, 2, 3].map(|k| v[16 + k] as f32 / 255.0);
                let sum: f32 = w.iter().sum();
                if sum > 0.0 {
                    w = w.map(|x| x / sum);
                } else {
                    w = [1.0, 0.0, 0.0, 0.0];
                }
                bones.push([0, 1, 2, 3].map(|k| remap(v[12 + k])));
                weights.push(w);
            } else {
                let n = if stride > 12 { v[12] } else { 0 };
                bones.push([remap(n), 0, 0, 0]);
                weights.push([1.0, 0.0, 0.0, 0.0]);
            }
        }
    }
    let uvs = match find(RES_VERTEX_BUFFERS, Some(1)) {
        Some(r) => {
            let b = slice(&data, base + r.offset, vertex_count * 8)?;
            (0..vertex_count)
                .map(|i| [f32_at(b, i * 8), f32_at(b, i * 8 + 4)])
                .collect()
        }
        None => vec![[0.0; 2]; vertex_count],
    };
    let lightmap_uvs = match find(RES_VERTEX_BUFFERS, Some(3)) {
        Some(r) if !layout.model && r.size >= vertex_count * 8 => {
            let b = slice(&data, base + r.offset, vertex_count * 8)?;
            (0..vertex_count)
                .map(|i| [f32_at(b, i * 8), f32_at(b, i * 8 + 4)])
                .collect()
        }
        _ => Vec::new(),
    };
    let normals = match find(RES_VERTEX_BUFFERS, Some(2)) {
        // normal, binormal, tangent: 3 x float3 per vertex
        Some(r) if r.size >= vertex_count * 36 => {
            let b = slice(&data, base + r.offset, vertex_count * 36)?;
            (0..vertex_count)
                .map(|i| {
                    [
                        f32_at(b, i * 36),
                        f32_at(b, i * 36 + 4),
                        f32_at(b, i * 36 + 8),
                    ]
                })
                .collect()
        }
        _ => vec![[0.0, 0.0, 1.0]; vertex_count],
    };

    let sub = slice(&data, base + submeshes.offset, submeshes.size)?;
    let mut parts = Vec::new();
    for s in sub.as_chunks::<SUBMESH_SIZE>().0 {
        let material = i16_at(s, 4);
        let start = u16::from_le_bytes([s[6], s[7]]) as usize;
        let len = u16::from_le_bytes([s[8], s[9]]) as usize;
        let Some(range) = idx.get(start..start + len) else {
            continue;
        };
        let tris = if is_list {
            range.iter().map(|&i| i as u32).collect()
        } else {
            strip_to_list(range)
        };
        let tris: Vec<u32> = tris
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|t| t.iter().all(|&i| (i as usize) < vertex_count))
            .flatten()
            .copied()
            .collect();
        if !tris.is_empty() {
            parts.push(Part {
                material,
                indices: tris,
            });
        }
    }
    Ok(Some(Section {
        positions,
        normals,
        uvs,
        parts,
        bones,
        weights,
        lightmap_uvs,
    }))
}

/// All render geometry of a structure BSP: clusters plus placed instanced geometry.
pub fn bsp_render_geometry(set: &mut MapSet, bsp: &StructureBsp) -> Result<LevelGeometry> {
    let region = bsp.region;
    let map = &mut set.map;
    let sbsp = map.read_in(region, bsp.bsp_address, 0x23C)?;
    let clusters = map.read_block(region, &sbsp, SBSP_CLUSTERS, CLUSTER_SIZE)?;
    let materials = map.read_block(region, &sbsp, SBSP_MATERIALS, MATERIAL_SIZE)?;
    let defs = map.read_block(region, &sbsp, SBSP_INSTANCE_DEFS, INSTANCE_DEF_SIZE)?;
    let instances = map.read_block(region, &sbsp, SBSP_INSTANCES, INSTANCE_SIZE)?;

    let shaders = materials
        .as_chunks::<MATERIAL_SIZE>()
        .0
        .iter()
        .map(|m| {
            let shader = DatumIndex(u32_at(m, 0xC));
            if shader == DatumIndex::NONE {
                DatumIndex(u32_at(m, 0x4))
            } else {
                shader
            }
        })
        .collect();

    let mut sections = Vec::new();
    let mut owners = Vec::new();
    for (i, c) in clusters.as_chunks::<CLUSTER_SIZE>().0.iter().enumerate() {
        if let Some(s) = read_section(set, Source::Map, region, SectionLayout::BSP, c)? {
            sections.push(s);
            owners.push(SectionOwner::Cluster(i));
        }
    }
    let mut def_sections = Vec::new();
    for d in defs.as_chunks::<INSTANCE_DEF_SIZE>().0 {
        def_sections.push(read_section(
            set,
            Source::Map,
            region,
            SectionLayout::BSP,
            d,
        )?);
    }
    let mut instance_vertices = Vec::new();
    for (i, inst) in instances.as_chunks::<INSTANCE_SIZE>().0.iter().enumerate() {
        let Some(Some(def)) = usize::try_from(i16_at(inst, 0x34))
            .ok()
            .and_then(|d| def_sections.get(d))
        else {
            instance_vertices.push(0);
            continue;
        };
        instance_vertices.push(def.positions.len());
        let v = |o| [f32_at(inst, o), f32_at(inst, o + 4), f32_at(inst, o + 8)];
        sections.push(def.transformed(f32_at(inst, 0), v(0x4), v(0x10), v(0x1C), v(0x28)));
        owners.push(SectionOwner::Instance(i));
    }
    Ok(LevelGeometry {
        sections,
        owners,
        instance_vertices,
        shaders,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_alternates_winding_and_skips_degenerates() {
        assert_eq!(strip_to_list(&[0, 1, 2, 3]), vec![0, 1, 2, 1, 3, 2]);
        assert_eq!(strip_to_list(&[0, 1, 2, 2, 3, 4]), vec![0, 1, 2, 2, 4, 3]);
        assert!(strip_to_list(&[0, 1]).is_empty());
    }
}
