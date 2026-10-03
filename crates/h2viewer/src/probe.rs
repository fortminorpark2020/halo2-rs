//! Lighting objects with the level's baked light: like Halo 2, an object
//! takes the light of the surface it stands on.

use crate::scene::MeshData;
use blam_cache::bitmap::Image;
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

/// Bilinear sample of an sRGB image at `uv` (clamped), in 0..1 per channel.
fn sample(img: &Image, uv: [f32; 2]) -> [f32; 3] {
    let (w, h) = (img.width.max(1) as f32, img.height.max(1) as f32);
    let x = (uv[0].clamp(0.0, 1.0) * w - 0.5).clamp(0.0, w - 1.0);
    let y = (uv[1].clamp(0.0, 1.0) * h - 0.5).clamp(0.0, h - 1.0);
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(img.width - 1), (y0 + 1).min(img.height - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let texel = |x: u32, y: u32| {
        let i = ((y * img.width + x) * 4) as usize;
        Vec3::new(
            img.rgba[i] as f32,
            img.rgba[i + 1] as f32,
            img.rgba[i + 2] as f32,
        ) / 255.0
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
            let indices = &level.indices[first..first + b.index_count as usize];
            for t in indices.as_chunks::<3>().0 {
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
    pub fn at(&self, textures: &[Image], p: Vec3) -> Option<[f32; 3]> {
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
            if let Some(img) = textures.get(page) {
                let lm = sample(img, uv);
                colour = [colour[0] * lm[0], colour[1] * lm[1], colour[2] * lm[2]];
            }
        }
        Some(colour)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Batch, Vertex};

    #[test]
    fn finds_the_floor_below() {
        let v = |x: f32, y: f32, z: f32, light: [f32; 4]| {
            let mut v = Vertex::new([x, y, z], [0.0, 0.0, 1.0], [0.0; 2]);
            v.light = light;
            v
        };
        let dark = [0.1, 0.1, 0.1, 0.0];
        let bright = [0.9, 0.8, 0.7, 0.0];
        let mesh = MeshData {
            vertices: vec![
                v(-5.0, -5.0, 0.0, bright),
                v(5.0, -5.0, 0.0, bright),
                v(0.0, 5.0, 0.0, bright),
                v(-5.0, -5.0, 3.0, dark),
                v(5.0, -5.0, 3.0, dark),
                v(0.0, 5.0, 3.0, dark),
            ],
            indices: vec![0, 1, 2, 3, 4, 5],
            batches: vec![Batch {
                material: 0,
                lightmap: 0,
                first_index: 0,
                index_count: 6,
            }],
            baked_lighting: true,
            ..MeshData::default()
        };
        let probe = LevelLight::new(&mesh);
        // Standing on the lower floor, under the upper one.
        let c = probe.at(&[], Vec3::new(0.0, 0.0, 1.0)).unwrap();
        assert!((c[0] - 0.9).abs() < 1e-5);
        // On the upper floor.
        let c = probe.at(&[], Vec3::new(0.0, 0.0, 3.5)).unwrap();
        assert!((c[0] - 0.1).abs() < 1e-5);
        assert!(probe.at(&[], Vec3::new(50.0, 0.0, 1.0)).is_none());
    }
}
