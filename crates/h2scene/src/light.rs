//! Lighting objects with the level's baked light: like Halo 2, an object
//! takes the light of the surface it stands on. Copied from h2viewer's
//! `probe.rs` (menu-preview).

use crate::data::{MeshData, TextureData};
use glam::{Vec2, Vec3};
use std::collections::HashMap;

/// Grid cell size (world units) for finding the triangles under a point.
const CELL: f32 = 2.0;
/// How far below a point to look for a floor.
const MAX_DROP: f32 = 30.0;

#[derive(Default)]
pub struct LevelLight {
    positions: Vec<Vec3>,
    lightmap_uv: Vec<[f32; 2]>,
    light: Vec<[f32; 4]>,
    /// Vertex indices and lightmap page texture of each triangle.
    triangles: Vec<([u32; 3], usize)>,
    grid: HashMap<(i32, i32), Vec<u32>>,
}

fn cell(x: f32, y: f32) -> (i32, i32) {
    ((x / CELL).floor() as i32, (y / CELL).floor() as i32)
}

/// Bilinear sample of an sRGB texture's full-size level at `uv` (clamped),
/// in 0..1 per channel.
fn sample(tex: &TextureData, uv: [f32; 2]) -> [f32; 3] {
    let (w, h) = (tex.w.max(1), tex.h.max(1));
    let Some(rgba) = tex
        .mips
        .first()
        .filter(|m| m.len() >= (w as usize) * (h as usize) * 4)
    else {
        return [1.0; 3];
    };
    let (fw, fh) = (w as f32, h as f32);
    let x = (uv[0].clamp(0.0, 1.0) * fw - 0.5).clamp(0.0, fw - 1.0);
    let y = (uv[1].clamp(0.0, 1.0) * fh - 0.5).clamp(0.0, fh - 1.0);
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let texel = |x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        Vec3::new(rgba[i] as f32, rgba[i + 1] as f32, rgba[i + 2] as f32) / 255.0
    };
    let top = texel(x0, y0).lerp(texel(x1, y0), fx);
    let bottom = texel(x0, y1).lerp(texel(x1, y1), fx);
    top.lerp(bottom, fy).into()
}

impl LevelLight {
    /// From the level mesh (whose vertices carry baked light).
    pub fn new(level: &MeshData) -> LevelLight {
        let mut probe = LevelLight {
            positions: level
                .vertices
                .iter()
                .map(|v| Vec3::from(v.position))
                .collect(),
            lightmap_uv: level.vertices.iter().map(|v| v.lightmap_uv).collect(),
            light: level.vertices.iter().map(|v| v.light).collect(),
            ..LevelLight::default()
        };
        if !level.baked_lighting {
            return probe;
        }
        for b in &level.batches {
            let first = b.first_index as usize;
            let Some(indices) = level.indices.get(first..first + b.index_count as usize) else {
                continue;
            };
            for t in indices.as_chunks::<3>().0 {
                if t.iter().any(|&i| i as usize >= probe.positions.len()) {
                    continue;
                }
                let id = probe.triangles.len() as u32;
                probe.triangles.push((*t, b.lightmap));
                let p = t.map(|i| probe.positions[i as usize]);
                let lo = cell(
                    p[0].x.min(p[1].x).min(p[2].x),
                    p[0].y.min(p[1].y).min(p[2].y),
                );
                let hi = cell(
                    p[0].x.max(p[1].x).max(p[2].x),
                    p[0].y.max(p[1].y).max(p[2].y),
                );
                for cx in lo.0..=hi.0 {
                    for cy in lo.1..=hi.1 {
                        probe.grid.entry((cx, cy)).or_default().push(id);
                    }
                }
            }
        }
        probe
    }

    /// Baked light on the surface below `p`, in the level's encoding
    /// (gamma space, half brightness), or `None` with nothing below.
    pub fn at(&self, textures: &[TextureData], p: Vec3) -> Option<[f32; 3]> {
        let ids = self.grid.get(&cell(p.x, p.y))?;
        let q = Vec2::new(p.x, p.y);
        let mut best: Option<(f32, u32, Vec3)> = None;
        for &id in ids {
            let (t, _) = self.triangles[id as usize];
            let [a, b, c] = t.map(|i| self.positions[i as usize]);
            // Barycentric coordinates of p in the triangle's xy projection.
            let (a2, b2, c2) = (a.truncate(), b.truncate(), c.truncate());
            let det = (b2 - a2).perp_dot(c2 - a2);
            if det.abs() < 1e-8 {
                continue;
            }
            let u = (q - a2).perp_dot(c2 - a2) / det;
            let v = (b2 - a2).perp_dot(q - a2) / det;
            if u < 0.0 || v < 0.0 || u + v > 1.0 {
                continue;
            }
            let z = a.z + u * (b.z - a.z) + v * (c.z - a.z);
            let drop = p.z - z;
            if !(-0.05..=MAX_DROP).contains(&drop) {
                continue;
            }
            if best.is_none_or(|(d, _, _)| drop < d) {
                best = Some((drop, id, Vec3::new(1.0 - u - v, u, v)));
            }
        }
        let (_, id, w) = best?;
        let (t, page) = self.triangles[id as usize];
        let [i, j, k] = t.map(|i| i as usize);
        let mix4 = |v: &[[f32; 4]]| {
            let f = |c: usize| v[i][c] * w.x + v[j][c] * w.y + v[k][c] * w.z;
            [f(0), f(1), f(2), f(3)]
        };
        let light = mix4(&self.light);
        let mut colour = [light[0], light[1], light[2]];
        if light[3] > 0.5 {
            let uv = [
                self.lightmap_uv[i][0] * w.x
                    + self.lightmap_uv[j][0] * w.y
                    + self.lightmap_uv[k][0] * w.z,
                self.lightmap_uv[i][1] * w.x
                    + self.lightmap_uv[j][1] * w.y
                    + self.lightmap_uv[k][1] * w.z,
            ];
            if let Some(tex) = textures.get(page) {
                let lm = sample(tex, uv);
                colour = [colour[0] * lm[0], colour[1] * lm[1], colour[2] * lm[2]];
            }
        }
        Some(colour)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Batch, Vertex};

    fn vertex(x: f32, y: f32, z: f32, light: [f32; 4]) -> Vertex {
        let mut v = Vertex::new([x, y, z], [0.0, 0.0, 1.0], [0.0; 2]);
        v.light = light;
        v
    }

    #[test]
    fn finds_the_floor_below() {
        let dark = [0.1, 0.1, 0.1, 0.0];
        let bright = [0.9, 0.8, 0.7, 0.0];
        let mesh = MeshData {
            vertices: vec![
                vertex(-5.0, -5.0, 0.0, bright),
                vertex(5.0, -5.0, 0.0, bright),
                vertex(0.0, 5.0, 0.0, bright),
                vertex(-5.0, -5.0, 3.0, dark),
                vertex(5.0, -5.0, 3.0, dark),
                vertex(0.0, 5.0, 3.0, dark),
            ],
            indices: vec![0, 1, 2, 3, 4, 5],
            batches: vec![Batch {
                material: 0,
                lightmap: 0,
                first_index: 0,
                index_count: 6,
            }],
            baked_lighting: true,
        };
        let probe = LevelLight::new(&mesh);
        // Standing on the lower floor, under the upper one.
        let c = probe.at(&[], Vec3::new(0.0, 0.0, 1.0)).unwrap();
        assert!((c[0] - 0.9).abs() < 1e-5);
        // On the upper floor.
        let c = probe.at(&[], Vec3::new(0.0, 0.0, 3.5)).unwrap();
        assert!((c[0] - 0.1).abs() < 1e-5);
        // Off the floors, and far above them.
        assert!(probe.at(&[], Vec3::new(50.0, 0.0, 1.0)).is_none());
        assert!(probe.at(&[], Vec3::new(0.0, 0.0, 50.0)).is_none());
    }

    #[test]
    fn a_lightmap_page_tints_the_floor() {
        // A floor whose vertices take their light from page 1, a 2x2
        // texture: red on the left half, blue on the right.
        let paged = [1.0; 4];
        let mut vs = vec![
            vertex(0.0, 0.0, 0.0, paged),
            vertex(10.0, 0.0, 0.0, paged),
            vertex(0.0, 10.0, 0.0, paged),
        ];
        vs[0].lightmap_uv = [0.0, 0.0];
        vs[1].lightmap_uv = [1.0, 0.0];
        vs[2].lightmap_uv = [0.0, 1.0];
        let mesh = MeshData {
            vertices: vs,
            indices: vec![0, 1, 2],
            batches: vec![Batch {
                material: 0,
                lightmap: 1,
                first_index: 0,
                index_count: 3,
            }],
            baked_lighting: true,
        };
        let red = [255, 0, 0, 255];
        let blue = [0, 0, 255, 255];
        let page = TextureData::new(2, 2, [red, blue, red, blue].concat()).unwrap();
        let textures = [TextureData::fallback(), page];
        let probe = LevelLight::new(&mesh);
        let left = probe.at(&textures, Vec3::new(0.5, 1.0, 0.2)).unwrap();
        assert!(left[0] > 0.9 && left[2] < 0.1, "{left:?}");
        let right = probe.at(&textures, Vec3::new(8.0, 1.0, 0.2)).unwrap();
        assert!(right[2] > 0.9 && right[0] < 0.1, "{right:?}");
        // A level without baked light has no floors to ask.
        let flat = MeshData {
            baked_lighting: false,
            ..mesh
        };
        assert!(LevelLight::new(&flat)
            .at(&textures, Vec3::new(0.5, 1.0, 0.2))
            .is_none());
    }
}
