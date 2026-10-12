//! Shader (`shad`) tags: which bitmaps a surface uses and how it is drawn.
//!
//! A shader names a template (`stem`) and fills the template's bitmap and
//! colour slots in its postprocess definition. Slots are positional, so the
//! common templates are described here by where their colour map, glow map
//! and glow colour sit.

use crate::mapset::MapSet;
use crate::{u32_at, DatumIndex, Result};

/// Runtime properties block: the engine's flattened view of the shader.
const SHAD_RUNTIME_PROPERTIES: usize = 12;
const RUNTIME_PROPERTIES_SIZE: usize = 80;
/// Postprocess definition: the bitmaps and colours the template's passes use.
const SHAD_POSTPROCESS: usize = 0x20;
const POSTPROCESS_SIZE: usize = 0x7C;
const POSTPROCESS_BITMAPS: usize = 0x4;
const POSTPROCESS_BITMAP_SIZE: usize = 0xC;
const POSTPROCESS_COLORS: usize = 0xC;
const POSTPROCESS_COLOR_SIZE: usize = 0x4;
/// Placeholder bitmaps templates fall back on; never a surface's own texture.
const DEFAULT_BITMAPS: &str = "shaders\\default_bitmaps\\";
const TEMPLATES: &str = "shaders\\shader_templates\\";

/// How a surface combines with what is behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Blend {
    #[default]
    Opaque,
    /// Opaque, with holes where the colour map's alpha is below half.
    AlphaTest,
    /// Blended by the colour map's (or mask's) alpha.
    Alpha,
    /// Added onto what is behind (glows, energy, light beams).
    Additive,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ShaderInfo {
    /// Template path under `shaders\shader_templates\`, e.g. `opaque\tex_bump`.
    pub template: String,
    /// Base colour texture, if the shader has one.
    pub diffuse: Option<DatumIndex>,
    pub blend: Blend,
    /// Self-illumination map and its colour.
    pub illum: Option<(DatumIndex, [f32; 3])>,
    /// Opacity map, for templates whose colour map carries no alpha.
    pub mask: Option<DatumIndex>,
    /// Change-colour map: where a player's primary (red channel) and
    /// secondary (green) armour colours tint the surface.
    pub change_color: Option<DatumIndex>,
    /// A second colour map multiplied into the first (colour and alpha),
    /// and how much the product is scaled by (the cloud templates double
    /// it).
    pub multiply: Option<(DatumIndex, f32)>,
    /// Colour multiplied into the base colour.
    pub tint: [f32; 3],
    /// Multiplies alpha-blended surfaces' opacity.
    pub opacity: f32,
}

/// A postprocess colour: bytes B, G, R, A.
fn color(c: &[u8]) -> [f32; 3] {
    [
        c[2] as f32 / 255.0,
        c[1] as f32 / 255.0,
        c[0] as f32 / 255.0,
    ]
}

/// A postprocess colour's alpha byte.
fn color_alpha(c: &[u8]) -> f32 {
    c[3] as f32 / 255.0
}

/// A shader as the levels draw it.
pub fn read_shader(set: &mut MapSet, shader: DatumIndex) -> Result<ShaderInfo> {
    read(set, shader, false)
}

/// A shader as the main menu's scene draws it: its sky's cloud templates
/// multiply in their second map, and plain glows (`opaque\illum`) show.
/// The levels keep their own look (`read_shader`) until they're checked
/// against Halo 2 the same way.
pub fn read_menu_shader(set: &mut MapSet, shader: DatumIndex) -> Result<ShaderInfo> {
    read(set, shader, true)
}

fn read(set: &mut MapSet, shader: DatumIndex, menu: bool) -> Result<ShaderInfo> {
    let (src, _, data) = set.tag_data(shader)?;
    let file = set.get(src);
    let region = file.meta_region();
    let props = file.read_block(
        region,
        &data,
        SHAD_RUNTIME_PROPERTIES,
        RUNTIME_PROPERTIES_SIZE,
    )?;
    let runtime_diffuse = props
        .get(..RUNTIME_PROPERTIES_SIZE)
        .map(|p| DatumIndex(u32_at(p, 4)))
        .filter(|d| *d != DatumIndex::NONE);
    let pp = file.read_block(region, &data, SHAD_POSTPROCESS, POSTPROCESS_SIZE)?;
    let (bitmaps, colors) = match pp.get(..POSTPROCESS_SIZE) {
        Some(p) => (
            file.read_block(region, p, POSTPROCESS_BITMAPS, POSTPROCESS_BITMAP_SIZE)?,
            file.read_block(region, p, POSTPROCESS_COLORS, POSTPROCESS_COLOR_SIZE)?,
        ),
        None => (Vec::new(), Vec::new()),
    };
    let bitmaps: Vec<DatumIndex> = bitmaps
        .as_chunks::<POSTPROCESS_BITMAP_SIZE>()
        .0
        .iter()
        .map(|b| DatumIndex(u32_at(b, 0)))
        .collect();
    let colors: Vec<([f32; 3], f32)> = colors
        .as_chunks::<POSTPROCESS_COLOR_SIZE>()
        .0
        .iter()
        .map(|c| (color(c), color_alpha(c)))
        .collect();
    let template = set
        .locate(DatumIndex(u32_at(&data, 4)))
        .map(|(_, t)| t.name.trim_start_matches(TEMPLATES).to_string())
        .unwrap_or_default();
    // Each slot's bitmap, unless it holds one of the engine's placeholders.
    let own: Vec<Option<DatumIndex>> = bitmaps
        .iter()
        .map(|&d| {
            set.locate(d)
                .is_some_and(|(_, t)| !t.name.starts_with(DEFAULT_BITMAPS))
                .then_some(d)
        })
        .collect();
    Ok(shader_info(template, runtime_diffuse, &own, &colors, menu))
}

/// What a shader's template makes of its slots: `own` holds each bitmap
/// slot's own bitmap (none for a placeholder) and `colors` each colour
/// slot's colour and alpha.
fn shader_info(
    template: String,
    runtime_diffuse: Option<DatumIndex>,
    own: &[Option<DatumIndex>],
    colors: &[([f32; 3], f32)],
    menu: bool,
) -> ShaderInfo {
    let first_own = own.iter().copied().flatten().next();
    let own = |i: usize| own.get(i).copied().flatten();
    let slot_color = |i: usize| colors.get(i).map_or([1.0; 3], |c| c.0);
    let slot_alpha = |i: usize| colors.get(i).map_or(1.0, |c| c.1);

    let mut info = ShaderInfo {
        diffuse: runtime_diffuse.or(first_own),
        tint: [1.0; 3],
        opacity: 1.0,
        ..ShaderInfo::default()
    };
    let t = template.as_str();
    match t {
        "opaque\\tex_bump_illum" | "opaque\\tex_bump_illum_bloom" => {
            info.illum = own(3).map(|b| (b, slot_color(2)));
        }
        "opaque\\tex_bump_env_illum" | "opaque\\tex_bump_env_illum_combined" => {
            info.illum = own(5).map(|b| (b, slot_color(6)));
        }
        // Three-channel glow maps light each channel with its own colour;
        // shown as the map's own colours for now.
        "opaque\\tex_bump_illum_3_channel" => info.illum = own(3).map(|b| (b, [1.0; 3])),
        "opaque\\tex_bump_env_illum_3_channel_occlusion_combined" => {
            info.illum = own(4).map(|b| (b, [1.0; 3]));
        }
        "opaque\\illum_3_channel" => {
            info.illum = own(0).map(|b| (b, [1.0; 3]));
            info.tint = [0.0; 3];
        }
        // Only a glow: its pass shows map 0 times the colour.
        "opaque\\illum" if menu => {
            info.illum = own(0).map(|b| (b, slot_color(0)));
            info.tint = [0.0; 3];
        }
        "opaque\\overlay" => info.blend = Blend::Alpha,
        // Never drawn (lights a model variant switches off).
        "opaque\\render_layer_disabled" => {
            info.blend = Blend::Alpha;
            info.opacity = 0.0;
        }
        // Where the colour and change-colour maps sit depends on the
        // template: after an active-camo bump map in the armour ones.
        _ if t.starts_with("opaque\\") && t.contains("change_color") => {
            let (base, change) = if t == "opaque\\tex_bump_one_change_color" {
                (1, 2)
            } else if t.starts_with("opaque\\tex_bump_env")
                && !["combined", "indexed", "multiply_map"]
                    .iter()
                    .any(|k| t.contains(k))
            {
                (2, 4)
            } else {
                (1, 3)
            };
            info.diffuse = own(base).or(info.diffuse);
            info.change_color = own(change);
        }
        "transparent\\lit\\transparent_lit_alpha_blend_two_change_color" => {
            info.diffuse = own(0);
            info.change_color = own(1);
            info.blend = Blend::Alpha;
        }
        "transparent\\one_alpha_env"
        | "transparent\\one_alpha_env_illum"
        | "transparent\\sky_one_alpha_env"
        | "transparent\\sky_one_alpha_env_illum" => {
            info.diffuse = own(2).or(first_own);
            info.blend = Blend::Alpha;
        }
        // Their passes multiply the two maps, colour and alpha.
        "transparent\\two_alpha_clouds" | "transparent\\sky_two_alpha_clouds" if menu => {
            info.diffuse = own(0);
            info.multiply = own(1).map(|b| (b, 1.0));
            info.blend = Blend::Alpha;
        }
        // The levels' clouds: the second map as a mask.
        "transparent\\two_alpha_clouds" | "transparent\\sky_two_alpha_clouds" => {
            info.diffuse = own(0);
            info.mask = own(1);
            info.blend = Blend::Alpha;
        }
        // Twice the two maps' product, added, as bright as the colour's
        // alpha says (none for lightning that only flashes when animated).
        "transparent\\two_add_clouds" | "transparent\\sky_two_add_clouds" if menu => {
            info.diffuse = own(0);
            info.multiply = own(1).map(|b| (b, 2.0));
            info.opacity = slot_alpha(0);
            info.blend = Blend::Additive;
        }
        "transparent\\plasma_alpha" => {
            info.diffuse = own(2).or(first_own);
            info.tint = slot_color(0);
            info.blend = Blend::Additive;
        }
        // The energy sword's blade: noise in the blade's colour, where its
        // mask (map 3) says.
        "transparent\\plasma_mask_offset" => {
            info.diffuse = own(0).or(first_own);
            info.mask = own(3);
            info.tint = slot_color(2);
            info.blend = Blend::Additive;
        }
        // Map 0 masks what maps 1 (and 2) add.
        _ if t.starts_with("transparent\\one_add") || t.starts_with("transparent\\sky_one_add") => {
            info.diffuse = own(1).or(first_own);
            info.mask = own(0).filter(|_| own(1).is_some());
            let c = slot_color(0);
            if c.iter().any(|&v| v > 0.0) {
                info.tint = c;
            }
            info.blend = Blend::Additive;
        }
        // Water's reflections and ripples aren't drawn yet: a tinted sheet,
        // faded at its edges where the template has a fade map.
        _ if t.starts_with("water\\") => {
            info.diffuse = None;
            info.mask = (t == "water\\water_edge_blend").then(|| own(0)).flatten();
            info.tint = [0.2, 0.32, 0.36];
            info.opacity = 0.8;
            info.blend = Blend::Alpha;
        }
        _ if t.starts_with("transparent\\") => {
            info.blend = Blend::Alpha;
        }
        _ if t.contains("alpha_test") => info.blend = Blend::AlphaTest,
        _ => {}
    }
    info.template = template;
    info
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postprocess_colors_are_bgra() {
        // Lockout's blue grav lift.
        let [r, g, b] = color(&[0xb0, 0x38, 0x0b, 0x00]);
        assert!(b > 0.6 && r < 0.05 && g > 0.2, "{r} {g} {b}");
    }

    #[test]
    fn menu_shaders_multiply_clouds_and_show_glows() {
        let (a, b) = (DatumIndex(0xE001_0001), DatumIndex(0xE002_0002));
        let own = [Some(a), Some(b)];
        let glow = [0.2, 0.4, 0.6];
        let info = |template: &str, alpha: f32, menu: bool| {
            shader_info(template.into(), None, &own, &[(glow, alpha)], menu)
        };
        // A plain glow: only the menu shows it, as map 0 in slot 0's
        // colour over no base colour.
        let level = info("opaque\\illum", 1.0, false);
        assert_eq!(
            (level.illum, level.tint, level.diffuse),
            (None, [1.0; 3], Some(a))
        );
        let menu = info("opaque\\illum", 1.0, true);
        assert_eq!((menu.illum, menu.tint), (Some((a, glow)), [0.0; 3]));
        for clouds in [
            "transparent\\two_alpha_clouds",
            "transparent\\sky_two_alpha_clouds",
        ] {
            // The levels' clouds use the second map as a mask; the menu's
            // multiply it in, colour and alpha.
            let level = info(clouds, 1.0, false);
            assert_eq!((level.mask, level.multiply), (Some(b), None));
            let menu = info(clouds, 1.0, true);
            assert_eq!((menu.mask, menu.multiply), (None, Some((b, 1.0))));
            for i in [level, menu] {
                assert_eq!(
                    (i.diffuse, i.blend, i.opacity),
                    (Some(a), Blend::Alpha, 1.0)
                );
            }
        }
        for clouds in [
            "transparent\\two_add_clouds",
            "transparent\\sky_two_add_clouds",
        ] {
            // The levels draw them as any transparent shader; the menu adds
            // twice the maps' product, as bright as slot 0's alpha.
            let level = info(clouds, 0.0, false);
            assert_eq!(
                (level.multiply, level.blend, level.opacity),
                (None, Blend::Alpha, 1.0)
            );
            for alpha in [0.0, 1.0] {
                let menu = info(clouds, alpha, true);
                assert_eq!(menu.diffuse, Some(a));
                assert_eq!(menu.multiply, Some((b, 2.0)));
                assert_eq!((menu.blend, menu.opacity), (Blend::Additive, alpha));
            }
        }
        // A placeholder in slot 0 leaves the colour map to the next own
        // bitmap, or to the runtime properties' when there is one.
        let held = shader_info(String::new(), None, &[None, Some(b)], &[], false);
        assert_eq!(held.diffuse, Some(b));
        let runtime = shader_info(String::new(), Some(a), &[None, Some(b)], &[], false);
        assert_eq!(runtime.diffuse, Some(a));
    }

    #[test]
    fn postprocess_alpha_is_the_last_byte() {
        // The main menu's cloud dome shows in full; its lightning not at rest.
        assert_eq!(color_alpha(&[0x00, 0x00, 0x00, 0xff]), 1.0);
        assert_eq!(color_alpha(&[0x00, 0x00, 0x00, 0x00]), 0.0);
    }
}
