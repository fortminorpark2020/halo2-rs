//! CPU-side scene: one vertex/index buffer, textures, and draw batches per texture.

use blam_cache::bitmap::{self, Image};
use blam_cache::geometry::Mesh;
use blam_cache::{render, shader, DatumIndex, MapSet, PlayerSpawn};
use std::collections::HashMap;
use std::path::Path;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

pub struct Batch {
    pub texture: usize,
    pub first_index: u32,
    pub index_count: u32,
}

pub struct Scene {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    /// Texture 0 is always a plain light-grey fallback.
    pub textures: Vec<Image>,
    pub batches: Vec<Batch>,
    pub spawn: Option<PlayerSpawn>,
    /// Collision geometry, kept for framing and (later) movement.
    pub collision: Mesh,
}

fn fallback_texture() -> Image {
    Image {
        width: 1,
        height: 1,
        rgba: vec![200, 200, 200, 255],
    }
}

impl Scene {
    pub fn load(path: &Path) -> Result<Scene, Box<dyn std::error::Error>> {
        let mut set = MapSet::open(path)?;
        if set.shared.is_none() {
            println!(
                "warning: shared.map not found next to the map; textures from it will be missing"
            );
        }
        let bsps = set.map.structure_bsps()?;
        let mut collision = Mesh::default();
        for bsp in &bsps {
            collision.append(&set.map.bsp_collision_mesh(bsp)?);
        }
        let spawn = set
            .map
            .player_spawns()
            .ok()
            .and_then(|s| s.first().copied());

        let mut scene = Scene {
            vertices: Vec::new(),
            indices: Vec::new(),
            textures: vec![fallback_texture()],
            batches: Vec::new(),
            spawn,
            collision,
        };

        // Render geometry grouped by texture.
        let mut by_texture: HashMap<usize, Vec<u32>> = HashMap::new();
        let mut texture_of_bitmap: HashMap<DatumIndex, usize> = HashMap::new();
        let mut texture_of_shader: HashMap<DatumIndex, usize> = HashMap::new();
        let mut failures = 0;
        for bsp in &bsps {
            let geo = match render::bsp_render_geometry(&mut set, bsp) {
                Ok(g) => g,
                Err(e) => {
                    println!(
                        "warning: render geometry unavailable ({e}); showing collision geometry"
                    );
                    continue;
                }
            };
            let mut material_texture = Vec::with_capacity(geo.shaders.len());
            for &sh in &geo.shaders {
                let tex = if let Some(&t) = texture_of_shader.get(&sh) {
                    t
                } else {
                    let t = match shader::read_shader(&mut set, sh) {
                        Ok(shader::ShaderInfo { diffuse: Some(b) }) => {
                            if let Some(&t) = texture_of_bitmap.get(&b) {
                                t
                            } else {
                                let t = match bitmap::read_bitmap(&mut set, b) {
                                    Ok(img) => {
                                        scene.textures.push(img);
                                        scene.textures.len() - 1
                                    }
                                    Err(_) => {
                                        failures += 1;
                                        0
                                    }
                                };
                                texture_of_bitmap.insert(b, t);
                                t
                            }
                        }
                        _ => 0,
                    };
                    texture_of_shader.insert(sh, t);
                    t
                };
                material_texture.push(tex);
            }
            for section in &geo.sections {
                let base = scene.vertices.len() as u32;
                for i in 0..section.positions.len() {
                    scene.vertices.push(Vertex {
                        position: section.positions[i],
                        normal: section.normals[i],
                        uv: section.uvs[i],
                    });
                }
                for part in &section.parts {
                    let tex = usize::try_from(part.material)
                        .ok()
                        .and_then(|m| material_texture.get(m))
                        .copied();
                    by_texture
                        .entry(tex.unwrap_or(0))
                        .or_default()
                        .extend(part.indices.iter().map(|i| i + base));
                }
            }
        }
        if failures > 0 {
            println!("warning: {failures} textures couldn't be decoded");
        }

        if by_texture.is_empty() {
            scene.use_collision_only();
        } else {
            let mut keys: Vec<usize> = by_texture.keys().copied().collect();
            keys.sort_unstable();
            for k in keys {
                let idx = &by_texture[&k];
                scene.batches.push(Batch {
                    texture: k,
                    first_index: scene.indices.len() as u32,
                    index_count: idx.len() as u32,
                });
                scene.indices.extend_from_slice(idx);
            }
        }
        Ok(scene)
    }

    /// Flat-shaded collision geometry with the fallback texture.
    fn use_collision_only(&mut self) {
        self.vertices.clear();
        self.indices.clear();
        for t in self.collision.indices.as_chunks::<3>().0 {
            let p = t.map(|i| glam::Vec3::from(self.collision.positions[i as usize]));
            let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or(glam::Vec3::Z);
            for v in p {
                self.indices.push(self.vertices.len() as u32);
                self.vertices.push(Vertex {
                    position: v.into(),
                    normal: n.into(),
                    uv: [0.0; 2],
                });
            }
        }
        self.batches = vec![Batch {
            texture: 0,
            first_index: 0,
            index_count: self.indices.len() as u32,
        }];
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// Full mip chain for an RGBA8 image by 2x2 box filtering.
pub fn mip_chain(img: &Image) -> Vec<(u32, u32, Vec<u8>)> {
    let mut levels = vec![(img.width, img.height, img.rgba.clone())];
    while let Some((w, h, prev)) = levels.last() {
        if *w == 1 && *h == 1 {
            break;
        }
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(w - 1);
                        let sy = (y * 2 + dy).min(h - 1);
                        sum += prev[((sy * w + sx) * 4 + c) as usize] as u32;
                    }
                    next[((y * nw + x) * 4 + c) as usize] = (sum / 4) as u8;
                }
            }
        }
        levels.push((nw, nh, next));
    }
    levels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_chain_reaches_1x1() {
        let img = Image {
            width: 4,
            height: 2,
            rgba: vec![255; 4 * 2 * 4],
        };
        let mips = mip_chain(&img);
        assert_eq!(
            mips.iter().map(|m| (m.0, m.1)).collect::<Vec<_>>(),
            vec![(4, 2), (2, 1), (1, 1)]
        );
        assert!(mips[2].2.iter().all(|&b| b == 255));
    }
}
