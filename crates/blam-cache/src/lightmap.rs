//! Baked level lighting from a BSP's lightmap (`ltmp`) tag.
//!
//! Each BSP cluster and most instanced pieces of geometry get their own page
//! in the lightmap bitmap, addressed by the section's second set of texture
//! coordinates. Instances without a page carry per-vertex colours instead,
//! stored in shared "geometry buckets".

use crate::mapset::{MapSet, Source};
use crate::{i16_at, i32_at, u32_at, DatumIndex, Error, Result, StructureBsp};

const LTMP_GROUPS: usize = 0x80;
const GROUP_SIZE: usize = 0x68;
const GROUP_BITMAP: usize = 0x18;
const GROUP_CLUSTER_RENDER_INFO: usize = 0x28;
const GROUP_BUCKETS: usize = 0x40;
const GROUP_INSTANCE_RENDER_INFO: usize = 0x48;
const GROUP_INSTANCE_BUCKET_REFS: usize = 0x50;
const RENDER_INFO_SIZE: usize = 4;
const BUCKET_SIZE: usize = 0x38;
const BUCKET_REF_SIZE: usize = 0xC;
const RESOURCE_SIZE: usize = 0x10;

/// Bucket flags: which per-vertex buffers a bucket holds.
const BUCKET_COLOR: i16 = 2;

#[derive(Debug, Clone, PartialEq)]
pub enum InstanceLighting {
    /// Image index in the lightmap bitmap.
    Lightmap(usize),
    /// One RGB colour (0..1) per vertex of the instance's geometry.
    VertexColors(Vec<[f32; 3]>),
    Unlit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LevelLighting {
    /// The bitmap tag holding every lightmap page.
    pub bitmap: Option<DatumIndex>,
    /// Lightmap page for each BSP cluster.
    pub clusters: Vec<Option<usize>>,
    pub instances: Vec<InstanceLighting>,
}

/// A geometry bucket's per-vertex colour buffer, decoded.
fn bucket_colors(
    set: &mut MapSet,
    src: Source,
    region: crate::Region,
    b: &[u8],
) -> Result<Vec<[f32; 3]>> {
    if i16_at(b, 0) & BUCKET_COLOR == 0 {
        return Ok(Vec::new());
    }
    let pointer = u32_at(b, 0xC);
    let block_size = i32_at(b, 0x10);
    let section_size = i32_at(b, 0x14);
    if block_size <= 0 || section_size < 0 {
        return Ok(Vec::new());
    }
    let resources = set.get(src).read_block(region, b, 0x1C, RESOURCE_SIZE)?;
    let data = set.read_resource(src, pointer, block_size as usize)?;
    // Resource offsets count from after the section header.
    let base = section_size as usize + 8;
    for r in resources.as_chunks::<RESOURCE_SIZE>().0 {
        let (sub, size, offset) = (i16_at(r, 6), i32_at(r, 8), i32_at(r, 12));
        if i16_at(r, 4) == 0 && sub == 1 && size > 0 {
            let start = base + offset.max(0) as usize;
            let bytes = data
                .get(start..start + size as usize)
                .ok_or_else(|| Error::Corrupt("lightmap colour buffer past its block".into()))?;
            // D3DCOLOR: B, G, R, A.
            return Ok(bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| {
                    [
                        c[2] as f32 / 255.0,
                        c[1] as f32 / 255.0,
                        c[0] as f32 / 255.0,
                    ]
                })
                .collect());
        }
    }
    Ok(Vec::new())
}

/// Lighting for every cluster and instance of a BSP. `instance_vertices`
/// gives each instance's vertex count, to cut its colours out of a bucket.
pub fn read_level_lighting(
    set: &mut MapSet,
    bsp: &StructureBsp,
    instance_vertices: &[usize],
) -> Result<LevelLighting> {
    let mut out = LevelLighting {
        bitmap: None,
        clusters: Vec::new(),
        instances: vec![InstanceLighting::Unlit; instance_vertices.len()],
    };
    if bsp.lightmap == DatumIndex::NONE {
        return Ok(out);
    }
    let (src, _, data) = set.tag_data(bsp.lightmap)?;
    let region = set.get(src).meta_region();
    let groups = set
        .get(src)
        .read_block(region, &data, LTMP_GROUPS, GROUP_SIZE)?;
    let Some(g) = groups.as_chunks::<GROUP_SIZE>().0.first().copied() else {
        return Ok(out);
    };
    let bitmap = DatumIndex(u32_at(&g, GROUP_BITMAP + 4));
    out.bitmap = (bitmap != DatumIndex::NONE).then_some(bitmap);
    let file = set.get(src);
    let page = |r: &[u8]| usize::try_from(i16_at(r, 0)).ok();
    out.clusters = file
        .read_block(region, &g, GROUP_CLUSTER_RENDER_INFO, RENDER_INFO_SIZE)?
        .as_chunks::<RENDER_INFO_SIZE>()
        .0
        .iter()
        .map(|r| page(r))
        .collect();
    let instance_info =
        file.read_block(region, &g, GROUP_INSTANCE_RENDER_INFO, RENDER_INFO_SIZE)?;
    let refs_raw = file.read_block(region, &g, GROUP_INSTANCE_BUCKET_REFS, BUCKET_REF_SIZE)?;
    let mut refs = Vec::new();
    for r in refs_raw.as_chunks::<BUCKET_REF_SIZE>().0 {
        let offsets = file.read_block(region, r, 0x4, 2)?;
        let first = offsets.as_chunks::<2>().0.first().map(|o| i16_at(o, 0));
        refs.push((i16_at(r, 2), first));
    }
    let buckets_raw = file.read_block(region, &g, GROUP_BUCKETS, BUCKET_SIZE)?;
    let mut buckets = Vec::new();
    for b in buckets_raw.as_chunks::<BUCKET_SIZE>().0 {
        buckets.push(bucket_colors(set, src, region, b).unwrap_or_default());
    }

    let info: Vec<&[u8; RENDER_INFO_SIZE]> = instance_info
        .as_chunks::<RENDER_INFO_SIZE>()
        .0
        .iter()
        .collect();
    for (i, lighting) in out.instances.iter_mut().enumerate() {
        if let Some(p) = info.get(i).and_then(|r| page(&r[..])) {
            *lighting = InstanceLighting::Lightmap(p);
            continue;
        }
        let Some(&(bucket, Some(offset))) = refs.get(i) else {
            continue;
        };
        let colors = usize::try_from(bucket).ok().and_then(|b| buckets.get(b));
        let (Some(colors), Ok(offset)) = (colors, usize::try_from(offset)) else {
            continue;
        };
        if let Some(c) = colors.get(offset..offset + instance_vertices[i]) {
            *lighting = InstanceLighting::VertexColors(c.to_vec());
        }
    }
    Ok(out)
}
