//! Level geometry extraction from structure BSPs.
//!
//! The collision BSP is stored entirely in tag data (no raw resources), so it
//! is the quickest route to a complete, recognizable level mesh. Surfaces are
//! polygons described by a winged-edge structure.

use crate::{f32_at, i16_at, CacheFile, Region, Result, StructureBsp};
use std::io::{Read, Seek};

/// Collision BSP block inside `sbsp` tag data.
const SBSP_COLLISION_BSP: usize = 0x14;
const COLLISION_BSP_SIZE: usize = 0x40;
/// Instanced geometry (rocks, pillars, props baked into the level).
const SBSP_INSTANCE_DEFS: usize = 0x138;
const INSTANCE_DEF_SIZE: usize = 0xC8;
const INSTANCE_DEF_COLLISION: usize = 0x70;
const SBSP_INSTANCES: usize = 0x140;
const INSTANCE_SIZE: usize = 0x58;

const SURFACE_INVISIBLE: u8 = 1 << 1;

#[derive(Debug, Default, Clone)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let first = *self.positions.first()?;
        Some(
            self.positions
                .iter()
                .fold((first, first), |(mut lo, mut hi), p| {
                    for i in 0..3 {
                        lo[i] = lo[i].min(p[i]);
                        hi[i] = hi[i].max(p[i]);
                    }
                    (lo, hi)
                }),
        )
    }

    pub fn append(&mut self, other: &Mesh) {
        let base = self.positions.len() as u32;
        self.positions.extend_from_slice(&other.positions);
        self.indices.extend(other.indices.iter().map(|i| i + base));
    }

    pub fn to_obj(&self) -> String {
        let mut s = String::with_capacity(self.positions.len() * 32 + self.indices.len() * 8);
        for p in &self.positions {
            s.push_str(&format!("v {} {} {}\n", p[0], p[1], p[2]));
        }
        for t in self.indices.chunks_exact(3) {
            s.push_str(&format!("f {} {} {}\n", t[0] + 1, t[1] + 1, t[2] + 1));
        }
        s
    }
}

/// Raw collision BSP arrays.
struct CollisionBsp {
    surfaces: Vec<u8>,
    edges: Vec<u8>,
    vertices: Vec<u8>,
}

impl<R: Read + Seek> CacheFile<R> {
    fn read_collision_bsp(
        &mut self,
        region: Region,
        parent: &[u8],
        at: usize,
    ) -> Result<CollisionBsp> {
        Ok(CollisionBsp {
            surfaces: self.read_block(region, parent, at + 0x28, 0x8)?,
            edges: self.read_block(region, parent, at + 0x30, 0xC)?,
            vertices: self.read_block(region, parent, at + 0x38, 0x10)?,
        })
    }

    /// Collision mesh of a structure BSP, including instanced geometry placed
    /// in world space.
    pub fn bsp_collision_mesh(&mut self, bsp: &StructureBsp) -> Result<Mesh> {
        let region = bsp.region;
        let sbsp = self.read_in(region, bsp.bsp_address, 0x23C)?;
        let mut mesh = Mesh::default();

        let blocks = self.read_block(region, &sbsp, SBSP_COLLISION_BSP, COLLISION_BSP_SIZE)?;
        for i in 0..blocks.len() / COLLISION_BSP_SIZE {
            let c = self.read_collision_bsp(region, &blocks, i * COLLISION_BSP_SIZE)?;
            mesh.append(&collision_to_mesh(&c));
        }

        let defs = self.read_block(region, &sbsp, SBSP_INSTANCE_DEFS, INSTANCE_DEF_SIZE)?;
        let mut def_meshes = Vec::new();
        for i in 0..defs.len() / INSTANCE_DEF_SIZE {
            let c = self.read_collision_bsp(
                region,
                &defs,
                i * INSTANCE_DEF_SIZE + INSTANCE_DEF_COLLISION,
            )?;
            def_meshes.push(collision_to_mesh(&c));
        }

        let instances = self.read_block(region, &sbsp, SBSP_INSTANCES, INSTANCE_SIZE)?;
        for inst in instances.chunks_exact(INSTANCE_SIZE) {
            let def = i16_at(inst, 0x34);
            let Some(local) = usize::try_from(def).ok().and_then(|d| def_meshes.get(d)) else {
                continue;
            };
            let scale = f32_at(inst, 0x0);
            let v = |o| [f32_at(inst, o), f32_at(inst, o + 4), f32_at(inst, o + 8)];
            let (fwd, left, up, pos) = (v(0x4), v(0x10), v(0x1C), v(0x28));
            let mut placed = local.clone();
            for p in &mut placed.positions {
                let [x, y, z] = *p;
                for k in 0..3 {
                    p[k] = pos[k] + scale * (x * fwd[k] + y * left[k] + z * up[k]);
                }
            }
            mesh.append(&placed);
        }
        Ok(mesh)
    }
}

/// Walk each surface's edge ring and fan-triangulate the resulting polygon.
fn collision_to_mesh(c: &CollisionBsp) -> Mesh {
    let mut mesh = Mesh {
        positions: c
            .vertices
            .chunks_exact(0x10)
            .map(|v| [f32_at(v, 0), f32_at(v, 4), f32_at(v, 8)])
            .collect(),
        indices: Vec::new(),
    };
    let edge_count = c.edges.len() / 0xC;
    let vert_count = mesh.positions.len();

    for (si, s) in c.surfaces.chunks_exact(0x8).enumerate() {
        if s[4] & SURFACE_INVISIBLE != 0 {
            continue;
        }
        let first = i16_at(s, 2) as u16 as usize;
        let mut ring = Vec::new();
        let mut e = first;
        loop {
            if e >= edge_count || ring.len() > 256 {
                ring.clear();
                break;
            }
            let ed = &c.edges[e * 0xC..e * 0xC + 0xC];
            let left = i16_at(ed, 8) as u16 as usize;
            let (vert, next) = if left == si {
                (i16_at(ed, 0), i16_at(ed, 4))
            } else {
                (i16_at(ed, 2), i16_at(ed, 6))
            };
            ring.push(vert as u16 as u32);
            e = next as u16 as usize;
            if e == first {
                break;
            }
        }
        if ring.len() < 3 || ring.iter().any(|&v| v as usize >= vert_count) {
            continue;
        }
        for k in 1..ring.len() - 1 {
            mesh.indices
                .extend_from_slice(&[ring[0], ring[k], ring[k + 1]]);
        }
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le16(v: &mut Vec<u8>, x: i16) {
        v.extend_from_slice(&x.to_le_bytes());
    }

    #[test]
    fn quad_surface_becomes_two_triangles() {
        let mut vertices = Vec::new();
        for (x, y) in [(0.0f32, 0.0f32), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            for c in [x, y, 0.0, 0.0] {
                vertices.extend_from_slice(&c.to_le_bytes());
            }
        }
        // Edge i runs vertex i -> i+1 with surface 0 on its left.
        let mut edges = Vec::new();
        for i in 0..4i16 {
            for x in [i, (i + 1) % 4, (i + 1) % 4, (i + 3) % 4, 0, -1] {
                le16(&mut edges, x);
            }
        }
        let mut surfaces = Vec::new();
        le16(&mut surfaces, 0);
        le16(&mut surfaces, 0);
        surfaces.extend_from_slice(&[0, 0, 0, 0]);

        let mesh = collision_to_mesh(&CollisionBsp { surfaces, edges, vertices });
        assert_eq!(mesh.positions.len(), 4);
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
    }
}
