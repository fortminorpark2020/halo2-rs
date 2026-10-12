//! The menu scene as plain data, ready for a GPU but needing none:
//! vertices, triangles grouped by material, materials, and textures with
//! their mip chains. Copied from h2viewer's `scene.rs` (menu-preview),
//! without what only a game needs (skinning, HUD, weapons, sounds).

use crate::flythrough::Flythrough;
use blam_cache::model::Atmosphere;
use blam_cache::render::Section;
use blam_cache::shader::Blend;
use glam::Mat4;
use std::collections::HashMap;

/// One vertex. `#[repr(C)]`: five fields of f32s, 56 bytes, in this order
/// (`MeshData::vertex_bytes` gives them as bytes for a vertex buffer).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// Level geometry: where the vertex samples its lightmap page.
    pub lightmap_uv: [f32; 2],
    /// Level geometry: baked light colour; alpha 1 multiplies it by the
    /// lightmap page.
    pub light: [f32; 4],
}

impl Vertex {
    /// Bytes a vertex takes.
    pub const SIZE: usize = std::mem::size_of::<Vertex>();

    pub fn new(position: [f32; 3], normal: [f32; 3], uv: [f32; 2]) -> Vertex {
        Vertex {
            position,
            normal,
            uv,
            lightmap_uv: [0.0; 2],
            light: [1.0, 1.0, 1.0, 0.0],
        }
    }
}

/// What a material's second texture (`Material::aux`) does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AuxKind {
    #[default]
    None,
    /// Self-illumination, scaled by `Material::illum_color`.
    Illum,
    /// Opacity.
    Mask,
    /// Where a player's armour colours go (red: primary, green: secondary).
    ChangeColor,
    /// A player's emblem picture (the surface's own texture is a stand-in).
    Emblem,
    /// A second colour map multiplying the first, colour and alpha.
    Multiply,
    /// A second colour map multiplying the first, doubled (added clouds).
    Multiply2x,
}

/// Which pass draws a material: the opaque pass (with depth writes, and
/// holes where `Blend::AlphaTest` says), then the blended ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Pass {
    Opaque,
    Alpha,
    Additive,
}

/// How a surface is drawn: its textures and how it blends.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Material {
    /// Its colour map (texture 0, the fallback, when it has none).
    pub texture: usize,
    /// Glow, opacity or second colour map (texture 0 when unused).
    pub aux: usize,
    pub aux_kind: AuxKind,
    pub blend: Blend,
    pub illum_color: [f32; 3],
    pub tint: [f32; 3],
    pub opacity: f32,
}

impl Material {
    /// The fallback texture, untinted.
    pub fn plain(texture: usize) -> Material {
        Material {
            texture,
            tint: [1.0; 3],
            opacity: 1.0,
            ..Material::default()
        }
    }

    pub fn pass(&self) -> Pass {
        match self.blend {
            Blend::Opaque | Blend::AlphaTest => Pass::Opaque,
            Blend::Alpha => Pass::Alpha,
            Blend::Additive => Pass::Additive,
        }
    }

    /// The factor its second map's product is scaled by, for the
    /// multiplying kinds.
    pub fn multiply(&self) -> Option<f32> {
        match self.aux_kind {
            AuxKind::Multiply => Some(1.0),
            AuxKind::Multiply2x => Some(2.0),
            _ => None,
        }
    }
}

/// Triangles that share a material and a lightmap page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Batch {
    pub material: usize,
    /// Lightmap page texture (texture 0 when unused).
    pub lightmap: usize,
    pub first_index: u32,
    pub index_count: u32,
}

/// How one section of level geometry is lit.
#[derive(Clone, Debug, PartialEq)]
pub enum SectionLight {
    /// Texture index of its lightmap page.
    Page(usize),
    /// One colour per vertex.
    Colors(Vec<[f32; 3]>),
    Unlit,
}

/// Light given to level geometry that has no baked lighting.
pub const UNLIT: [f32; 4] = [0.5, 0.5, 0.5, 0.0];

/// A mesh: triangle lists by material.
#[derive(Clone, Debug, Default)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub batches: Vec<Batch>,
    /// Lit by its vertices' baked light rather than the object lighting.
    pub baked_lighting: bool,
}

impl MeshData {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Build from sections whose parts index `materials` (the model's
    /// shaders, mapped to scene materials). Level geometry passes each
    /// section's baked lighting in `lights`.
    pub fn from_sections<'a>(
        sections: impl IntoIterator<Item = &'a Section>,
        materials: &[usize],
        lights: &[SectionLight],
    ) -> MeshData {
        let mut mesh = MeshData {
            baked_lighting: !lights.is_empty(),
            ..MeshData::default()
        };
        let mut by_material: HashMap<(usize, usize), Vec<u32>> = HashMap::new();
        for (s, section) in sections.into_iter().enumerate() {
            let base = mesh.vertices.len() as u32;
            let count = section.positions.len();
            let mut light = lights.get(s).cloned();
            if matches!(light, Some(SectionLight::Page(_))) && section.lightmap_uvs.len() < count {
                light = Some(SectionLight::Unlit);
            }
            let page = match light {
                Some(SectionLight::Page(t)) => t,
                _ => 0,
            };
            for i in 0..count {
                // Short normal or uv lists (a damaged section) give zeros
                // rather than a panic.
                let normal = section.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]);
                let uv = section.uvs.get(i).copied().unwrap_or_default();
                let mut v = Vertex::new(section.positions[i], normal, uv);
                match &light {
                    Some(SectionLight::Page(_)) => {
                        v.lightmap_uv = section.lightmap_uvs[i];
                        v.light = [1.0; 4];
                    }
                    Some(SectionLight::Colors(c)) => {
                        v.light = c.get(i).map_or(UNLIT, |c| [c[0], c[1], c[2], 0.0]);
                    }
                    Some(SectionLight::Unlit) => v.light = UNLIT,
                    None => {}
                }
                mesh.vertices.push(v);
            }
            for part in &section.parts {
                let material = usize::try_from(part.material)
                    .ok()
                    .and_then(|m| materials.get(m))
                    .copied()
                    .unwrap_or(0);
                // Indices past the section's vertices (a damaged section)
                // drop their triangle.
                let tris = part.indices.as_chunks::<3>().0;
                by_material.entry((material, page)).or_default().extend(
                    tris.iter()
                        .filter(|t| t.iter().all(|&i| (i as usize) < count))
                        .flatten()
                        .map(|i| i + base),
                );
            }
        }
        // In material order: a model's materials follow its shader list,
        // which is the order skies layer their transparent parts in.
        let mut keys: Vec<(usize, usize)> = by_material.keys().copied().collect();
        keys.sort_unstable();
        for k in keys {
            let idx = &by_material[&k];
            if idx.is_empty() {
                continue;
            }
            mesh.batches.push(Batch {
                material: k.0,
                lightmap: k.1,
                first_index: mesh.indices.len() as u32,
                index_count: idx.len() as u32,
            });
            mesh.indices.extend_from_slice(idx);
        }
        mesh
    }

    /// Append `other`, its indices and batches moved past this mesh's.
    pub fn append(&mut self, other: MeshData) {
        let base = self.vertices.len() as u32;
        let first = self.indices.len() as u32;
        self.vertices.extend_from_slice(&other.vertices);
        self.indices.extend(other.indices.iter().map(|i| i + base));
        self.batches
            .extend(other.batches.into_iter().map(|b| Batch {
                first_index: b.first_index + first,
                ..b
            }));
    }

    /// The vertices as a vertex buffer's bytes: `Vertex`'s fields in
    /// order, little-endian f32s.
    pub fn vertex_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.vertices.len() * Vertex::SIZE);
        for v in &self.vertices {
            let fields = v
                .position
                .iter()
                .chain(&v.normal)
                .chain(&v.uv)
                .chain(&v.lightmap_uv)
                .chain(&v.light);
            for f in fields {
                out.extend_from_slice(&f.to_le_bytes());
            }
        }
        out
    }

    /// The indices as an index buffer's bytes (u32, little-endian).
    pub fn index_bytes(&self) -> Vec<u8> {
        self.indices.iter().flat_map(|i| i.to_le_bytes()).collect()
    }
}

/// An RGBA8 texture (sRGB colour, as the map's bitmaps decode) and its mip
/// chain down to 1x1: `mips[0]` is `w` by `h`, each next level half as big
/// (at least 1).
#[derive(Clone, Debug, PartialEq)]
pub struct TextureData {
    pub w: u32,
    pub h: u32,
    pub mips: Vec<Vec<u8>>,
}

impl TextureData {
    /// From a full-size RGBA8 image, with its mip chain; None when the
    /// pixels don't fill `w` by `h`.
    pub fn new(w: u32, h: u32, rgba: Vec<u8>) -> Option<TextureData> {
        if w == 0 || h == 0 || rgba.len() != (w as usize) * (h as usize) * 4 {
            return None;
        }
        Some(TextureData {
            w,
            h,
            mips: mip_chain(w, h, rgba),
        })
    }

    /// The texture used where a surface has none: one light grey pixel.
    pub fn fallback() -> TextureData {
        TextureData {
            w: 1,
            h: 1,
            mips: vec![vec![200, 200, 200, 255]],
        }
    }

    /// The size of mip level `level`.
    pub fn size(&self, level: usize) -> (u32, u32) {
        let shrink = |v: u32| v.checked_shr(level as u32).unwrap_or(0).max(1);
        (shrink(self.w), shrink(self.h))
    }
}

/// The full mip chain of an RGBA8 image by 2x2 box filtering: the image
/// itself, then each level half the size of the last, down to 1x1.
pub fn mip_chain(w: u32, h: u32, rgba: Vec<u8>) -> Vec<Vec<u8>> {
    let mut levels = vec![rgba];
    let (mut w, mut h) = (w, h);
    while w > 1 || h > 1 {
        let prev = levels.last().expect("starts with the image");
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
        levels.push(next);
        (w, h) = (nw, nh);
    }
    levels
}

/// A piece of scenery (the carrier, the flak guns...), drawn with the
/// level's light where it stands.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    /// Index into `SceneData::meshes`.
    pub mesh: usize,
    pub transform: Mat4,
    /// Baked light under the object (`LevelLight::at`), in the level's
    /// encoding; None with nothing below it.
    pub light: Option<[f32; 3]>,
    /// Its object tag's name, for logs.
    pub name: String,
}

/// mainmenu.map's scene: what's drawn behind the start screen and main
/// menu, and the camera path through it. Everything is plain data that can
/// be built on any thread and sent to the one that draws.
#[derive(Clone, Debug)]
pub struct SceneData {
    /// Every BSP's render geometry in one mesh, lit by its baked light.
    pub level: MeshData,
    /// The BSPs read.
    pub bsps: usize,
    /// The scenery's and the sky's meshes (`objects` and `sky` index it).
    pub meshes: Vec<MeshData>,
    pub objects: Vec<Object>,
    /// The sky's model, drawn around the camera before the level.
    pub sky: Option<usize>,
    /// The sky's fog and clear colour.
    pub atmosphere: Atmosphere,
    /// Texture 0 is the fallback.
    pub textures: Vec<TextureData>,
    /// Material 0 is the fallback.
    pub materials: Vec<Material>,
    /// The camera path, from the map's script.
    pub flythrough: Flythrough,
    /// Bitmaps that couldn't be decoded (their surfaces use texture 0).
    pub failed_textures: usize,
}

impl SceneData {
    /// Triangles in the level, the scenery as placed, and the sky.
    pub fn triangles(&self) -> (usize, usize, usize) {
        let mesh = |m: usize| self.meshes.get(m).map_or(0, MeshData::triangle_count);
        (
            self.level.triangle_count(),
            self.objects.iter().map(|o| mesh(o.mesh)).sum(),
            self.sky.map_or(0, mesh),
        )
    }

    /// Materials drawn in each pass.
    pub fn materials_by_pass(&self) -> [(Pass, usize); 3] {
        let count = |p| self.materials.iter().filter(|m| m.pass() == p).count();
        [Pass::Opaque, Pass::Alpha, Pass::Additive].map(|p| (p, count(p)))
    }

    /// Bytes the textures take with their mips.
    pub fn texture_bytes(&self) -> usize {
        self.textures
            .iter()
            .flat_map(|t| &t.mips)
            .map(Vec::len)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blam_cache::render::Part;

    #[test]
    fn mip_chain_reaches_1x1() {
        let mips = mip_chain(4, 2, vec![255; 4 * 2 * 4]);
        let sizes: Vec<usize> = mips.iter().map(Vec::len).collect();
        assert_eq!(sizes, vec![4 * 2 * 4, 2 * 4, 4]);
        assert!(mips[2].iter().all(|&b| b == 255));
        // Odd sizes round down, never below 1.
        let t = TextureData::new(5, 3, vec![0; 5 * 3 * 4]).unwrap();
        assert_eq!(t.mips.len(), 3);
        let sizes: Vec<(u32, u32)> = (0..t.mips.len()).map(|l| t.size(l)).collect();
        assert_eq!(sizes, vec![(5, 3), (2, 1), (1, 1)]);
        for (l, m) in t.mips.iter().enumerate() {
            let (w, h) = t.size(l);
            assert_eq!(m.len(), (w * h * 4) as usize);
        }
        // A 1x1 is its own chain.
        assert_eq!(TextureData::fallback().mips.len(), 1);
        assert_eq!(mip_chain(1, 1, vec![1, 2, 3, 4]), vec![vec![1, 2, 3, 4]]);
    }

    #[test]
    fn mips_average_their_four_pixels() {
        // A 2x2 of black and white averages to grey.
        let px = |v: u8| [v, v, v, 255];
        let rgba: Vec<u8> = [px(0), px(255), px(255), px(0)].concat();
        let t = TextureData::new(2, 2, rgba).unwrap();
        assert_eq!(t.mips[1], vec![127, 127, 127, 255]);
    }

    #[test]
    fn short_pixels_are_no_texture() {
        assert!(TextureData::new(4, 4, vec![0; 10]).is_none());
        assert!(TextureData::new(0, 4, Vec::new()).is_none());
    }

    fn section(parts: Vec<Part>, lightmap_uvs: Vec<[f32; 2]>, n: usize) -> Section {
        Section {
            positions: (0..n).map(|i| [i as f32, 0.0, 0.0]).collect(),
            normals: vec![[0.0, 0.0, 1.0]; n],
            uvs: vec![[0.0; 2]; n],
            parts,
            bones: Vec::new(),
            weights: Vec::new(),
            lightmap_uvs,
        }
    }

    fn part(material: i16, indices: Vec<u32>) -> Part {
        Part { material, indices }
    }

    #[test]
    fn sections_batch_by_texture() {
        let s = section(
            vec![part(1, vec![0, 1, 2]), part(0, vec![1, 2, 3])],
            Vec::new(),
            4,
        );
        let mesh = MeshData::from_sections([&s], &[5, 7], &[]);
        assert!(!mesh.baked_lighting);
        assert_eq!(mesh.batches.len(), 2);
        assert_eq!(mesh.batches[0].material, 5);
        assert_eq!(&mesh.indices[..3], &[1, 2, 3]);
        assert_eq!(mesh.batches[1].material, 7);
        assert_eq!(mesh.batches[1].first_index, 3);
        assert_eq!(mesh.triangle_count(), 2);
    }

    #[test]
    fn sections_share_batches_and_offset_their_vertices() {
        // Two sections with the same material: one batch, the second's
        // indices moved past the first's vertices. A part with a material
        // the model doesn't list takes material 0.
        let a = section(vec![part(0, vec![0, 1, 2])], Vec::new(), 3);
        let b = section(
            vec![part(0, vec![2, 1, 0]), part(9, vec![0, 1, 2])],
            Vec::new(),
            3,
        );
        let mesh = MeshData::from_sections([&a, &b], &[4], &[]);
        assert_eq!(mesh.vertices.len(), 6);
        let batches: Vec<(usize, u32)> = mesh
            .batches
            .iter()
            .map(|b| (b.material, b.index_count))
            .collect();
        assert_eq!(batches, vec![(0, 3), (4, 6)]);
        assert_eq!(&mesh.indices[3..], &[0, 1, 2, 5, 4, 3]);
    }

    #[test]
    fn damaged_sections_drop_bad_triangles() {
        // An index past the vertices drops its triangle; short normals and
        // uvs don't panic.
        let mut s = section(vec![part(0, vec![0, 1, 2, 1, 2, 7])], Vec::new(), 3);
        s.normals.truncate(1);
        s.uvs.clear();
        let mesh = MeshData::from_sections([&s], &[3], &[]);
        assert_eq!(mesh.indices, vec![0, 1, 2]);
        assert_eq!(mesh.vertices[2].normal, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn level_sections_carry_their_lighting() {
        let tri = || vec![part(0, vec![0, 1, 2])];
        let paged = section(tri(), vec![[0.25, 0.5]; 3], 3);
        let coloured = section(tri(), Vec::new(), 3);
        let missing_uvs = section(tri(), Vec::new(), 3);
        let mesh = MeshData::from_sections(
            [&paged, &coloured, &missing_uvs],
            &[4],
            &[
                SectionLight::Page(9),
                SectionLight::Colors(vec![[0.1, 0.2, 0.3]; 3]),
                SectionLight::Page(9),
            ],
        );
        assert!(mesh.baked_lighting);
        assert_eq!(mesh.vertices[0].light, [1.0; 4]);
        assert_eq!(mesh.vertices[0].lightmap_uv, [0.25, 0.5]);
        assert_eq!(mesh.vertices[3].light, [0.1, 0.2, 0.3, 0.0]);
        assert_eq!(mesh.vertices[6].light, UNLIT);
        let pages: Vec<usize> = mesh.batches.iter().map(|b| b.lightmap).collect();
        assert_eq!(pages, vec![0, 9]);
    }

    #[test]
    fn append_moves_indices_and_batches() {
        let s = section(vec![part(0, vec![0, 1, 2])], Vec::new(), 3);
        let mut a = MeshData::from_sections([&s], &[1], &[]);
        a.append(MeshData::from_sections([&s], &[2], &[]));
        assert_eq!(a.indices, vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(a.batches[1].first_index, 3);
        assert_eq!(a.batches[1].material, 2);
    }

    #[test]
    fn vertex_bytes_follow_the_field_order() {
        assert_eq!(Vertex::SIZE, 56);
        let mut v = Vertex::new([1.0, 2.0, 3.0], [0.0, 0.0, 1.0], [0.5, 0.25]);
        v.light = [0.1, 0.2, 0.3, 1.0];
        let mesh = MeshData {
            vertices: vec![v],
            indices: vec![0, 0, 0],
            ..MeshData::default()
        };
        let bytes = mesh.vertex_bytes();
        assert_eq!(bytes.len(), Vertex::SIZE);
        let f = |k: usize| f32::from_le_bytes(bytes[k * 4..k * 4 + 4].try_into().unwrap());
        assert_eq!([f(0), f(1), f(2)], [1.0, 2.0, 3.0]);
        assert_eq!([f(6), f(7)], [0.5, 0.25]);
        assert_eq!([f(8), f(9)], [0.0, 0.0]);
        assert_eq!(f(13), 1.0);
        assert_eq!(mesh.index_bytes().len(), 12);
    }

    #[test]
    fn materials_pick_their_pass() {
        let with = |blend| Material {
            blend,
            ..Material::plain(0)
        };
        assert_eq!(with(Blend::Opaque).pass(), Pass::Opaque);
        assert_eq!(with(Blend::AlphaTest).pass(), Pass::Opaque);
        assert_eq!(with(Blend::Alpha).pass(), Pass::Alpha);
        assert_eq!(with(Blend::Additive).pass(), Pass::Additive);
        let clouds = Material {
            aux_kind: AuxKind::Multiply2x,
            ..Material::plain(0)
        };
        assert_eq!(clouds.multiply(), Some(2.0));
        assert_eq!(Material::plain(0).multiply(), None);
    }
}
