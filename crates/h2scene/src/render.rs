//! Drawing the menu scene with wgpu, offscreen: no window and no surface,
//! the frame read back as pixels for a CPU canvas (the launcher's lobby
//! copies it in under its start screen and main menu). The parts of
//! menu-preview's h2viewer `gpu.rs` (ours) the scene needs: its mesh
//! shader without the players' camouflage, overshield, emblems or HUD,
//! the five mesh pipelines, the sky pass then the world pass, and a last
//! pass of our own, the veil, that darkens and tints the picture navy as
//! the menus show it.
//!
//! Nothing here panics on a GPU problem: a missing adapter, a device that
//! can't be made, a validation error or a lost device is an `Err`, and
//! the caller falls back to its still background.

use crate::camera::{look_to, Lens, FAR, NEAR};
use crate::data::{AuxKind, Material, MeshData, Object, SceneData, TextureData, Vertex};
use crate::flythrough::{Flythrough, FOV};
use blam_cache::model::Atmosphere;
use blam_cache::shader::Blend;
use glam::{Mat4, Vec3};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wgpu::util::DeviceExt;

/// The sky's own near and far planes (h2viewer's): it is drawn around the
/// camera, before the level, with a depth buffer of its own.
pub const SKY_NEAR: f32 = 1.0;
pub const SKY_FAR: f32 = 10_000.0;

/// How long a frame may take on the GPU before the renderer counts as
/// failed. An estimate: far longer than any frame should take.
const GPU_WAIT: Duration = Duration::from_secs(5);

/// Bytes between draws in the uniform buffer (each draw's `DrawUniforms`
/// at its own dynamic offset; 256 is the largest alignment wgpu asks).
const SLOT: u64 = 256;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// What the scene is drawn into, before the veil: the veil reads it.
const SCENE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// What the frame is cleared to where the sky's tag gives no colour
/// (h2viewer's sky blue, linear).
const DEFAULT_CLEAR: [f32; 3] = [0.62, 0.70, 0.80];

/// An sRGB byte as linear light.
pub fn srgb_to_linear(c: u8) -> f32 {
    srgb_decode(f32::from(c) / 255.0)
}

/// Linear light as an sRGB byte (rounded, clamped).
pub fn linear_to_srgb(c: f32) -> u8 {
    (srgb_encode(c) * 255.0).round() as u8
}

/// Linear light (0 to 1, clamped) as its sRGB-encoded value (0 to 1).
pub fn srgb_encode(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// An sRGB-encoded value (0 to 1, clamped) as linear light.
pub fn srgb_decode(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The last pass over the scene, as the menus darken and tint the
/// flythrough navy behind the logo and rows. It works on sRGB-encoded
/// values, as Halo 2 Vista's Direct3D 9 blending did (its screen's hue is
/// the same at every brightness in sRGB bytes, which a multiply in linear
/// light can't give): each pixel keeps `saturation` of its colour (the
/// rest is its grey), is brightened by `gain` and clipped at white, is
/// multiplied by one colour, then has another added. Halo 2 Vista's own
/// numbers for this aren't known; `Veil::menu`'s are estimates chosen
/// against the owner's Cartographer screenshot
/// (docs/notes/launcher/menu.md, "What Phase 2.5 built").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Veil {
    /// Factors per channel, on sRGB-encoded values.
    pub multiply: [f32; 3],
    /// Added per channel after the multiply, sRGB-encoded.
    pub add: [f32; 3],
    /// How much of each pixel's colour is kept before the multiply: 1 all
    /// of it, 0 none (its grey alone).
    pub saturation: f32,
    /// How much each pixel is brightened before the multiply (what passes
    /// white is clipped there, so white comes out as the multiply).
    pub gain: f32,
}

/// Linear light's luminance weights (Rec. 709), for a pixel's grey (its
/// luminance, then encoded).
pub const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

impl Veil {
    /// Leaves the picture as it is.
    pub const NONE: Veil = Veil {
        multiply: [1.0; 3],
        add: [0.0; 3],
        saturation: 1.0,
        gain: 1.0,
    };

    /// A veil that keeps `saturation` of the colour, brightens it by
    /// `gain` (clipped at white), multiplies by `navy` (sRGB bytes) and
    /// adds `navy` times `k`.
    pub fn navy(navy: [u8; 3], k: f32, saturation: f32, gain: f32) -> Veil {
        let m = navy.map(|c| f32::from(c) / 255.0);
        Veil {
            multiply: m,
            add: m.map(|c| c * k.max(0.0)),
            saturation: saturation.clamp(0.0, 1.0),
            gain: gain.max(0.0),
        }
    }

    /// The menus' veil, by estimate (`MENU_NAVY`, `MENU_K`,
    /// `MENU_SATURATION`, `MENU_GAIN`).
    pub fn menu() -> Veil {
        Veil::navy(MENU_NAVY, MENU_K, MENU_SATURATION, MENU_GAIN)
    }

    /// `r,g,b,k`, `r,g,b,k,s` or `r,g,b,k,s,gain` (the navy as sRGB
    /// bytes, how much of it is added, the colour kept and the
    /// brightening, `MENU_SATURATION` and `MENU_GAIN` when left out), as
    /// `H2LOBBY_VEIL` gives it; None if it doesn't read.
    pub fn parse(text: &str) -> Option<Veil> {
        let parts: Vec<&str> = text.split(',').map(str::trim).collect();
        let (r, g, b, k, s, gain) = match parts.as_slice() {
            [r, g, b, k] => (r, g, b, k, None, None),
            [r, g, b, k, s] => (r, g, b, k, Some(s), None),
            [r, g, b, k, s, gain] => (r, g, b, k, Some(s), Some(gain)),
            _ => return None,
        };
        let byte = |s: &str| s.parse::<u8>().ok();
        let number = |s: &str| s.parse::<f32>().ok().filter(|v| v.is_finite() && *v >= 0.0);
        let k = number(k)?;
        let s = match s {
            Some(s) => number(s).filter(|s| *s <= 1.0)?,
            None => MENU_SATURATION,
        };
        let gain = match gain {
            Some(g) => number(g)?,
            None => MENU_GAIN,
        };
        Some(Veil::navy([byte(r)?, byte(g)?, byte(b)?], k, s, gain))
    }

    /// A linear colour after the veil (the GPU's pass, on the CPU).
    pub fn apply(&self, linear: [f32; 3]) -> [f32; 3] {
        let grey = srgb_encode((0..3).map(|c| linear[c] * LUMA[c]).sum());
        let mut out = [0.0; 3];
        for c in 0..3 {
            let kept = grey + (srgb_encode(linear[c]) - grey) * self.saturation;
            let lit = (kept * self.gain).clamp(0.0, 1.0);
            out[c] = srgb_decode(lit * self.multiply[c] + self.add[c]);
        }
        out
    }

    /// An sRGB pixel after the veil, as an sRGB pixel.
    pub fn apply_srgb(&self, px: [u8; 3]) -> [u8; 3] {
        self.apply(px.map(srgb_to_linear)).map(linear_to_srgb)
    }

    /// The veil pass's uniform: the multiply with the saturation, then the
    /// add with the gain.
    fn uniform(&self) -> [f32; 8] {
        let ([mr, mg, mb], [ar, ag, ab]) = (self.multiply, self.add);
        [mr, mg, mb, self.saturation, ar, ag, ab, self.gain]
    }
}

/// The menus' navy, as sRGB bytes: the colour the owner's Cartographer
/// screenshot clips at (an eighth of its upper half, the lit carrier,
/// the lift's beam and the flak, is exactly it, and nothing is
/// brighter), so white comes out as it. Its darker pixels are the same hue, scaled.
pub const MENU_NAVY: [u8; 3] = [44, 55, 88];
/// How much of the navy is added after the multiply: none (the
/// screenshot's shadows go down to about (6, 6, 10)). An estimate.
pub const MENU_K: f32 = 0.0;
/// How much colour the scene keeps: none. The screenshot is all one hue
/// (the brown sky and the water both navy); multiplying the colours
/// alone left the sky brown-black and the water purple. An estimate.
pub const MENU_SATURATION: f32 = 0.0;
/// How much the scene's grey (sRGB-encoded) is brightened before the
/// multiply. An estimate, fitted in the container (on lavapipe, with
/// Cartographer's mainmenu.map, a frame every 5 s of the loop) to the
/// screenshot's scene, away from the logo, the rows and the text: its
/// mean is about (28, 35, 56) and 8 % of it is clipped at the navy; 20
/// frames at 1280x720 came to (28, 35, 57) with 7 % clipped, and about
/// the same spread of brightness.
pub const MENU_GAIN: f32 = 2.15;

/// Where the camera is and how it faces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Eye {
    pub at: Vec3,
    pub forward: Vec3,
    pub up: Vec3,
}

impl Eye {
    /// The flythrough's camera `seconds` into the menus.
    pub fn flythrough(fly: &Flythrough, seconds: f32) -> Eye {
        let (at, forward, up) = fly.camera(seconds);
        Eye { at, forward, up }
    }

    /// The sky's view-projection and the world's, for a view `aspect`
    /// wide, through the menus' lens at the flythrough's field of view.
    pub fn view_projs(&self, aspect: f32) -> (Mat4, Mat4) {
        let lens = Lens::menu();
        let zoom = lens.fov_magnification(aspect, FOV);
        let sky = lens.projection(aspect, zoom, SKY_NEAR, SKY_FAR)
            * look_to(Vec3::ZERO, self.forward, self.up);
        let world =
            lens.projection(aspect, zoom, NEAR, FAR) * look_to(self.at, self.forward, self.up);
        (sky, world)
    }
}

/// What the renderer draws: a scene's meshes, textures and materials,
/// borrowed (`SceneData::parts`, or made up in the tests).
#[derive(Clone, Copy)]
pub struct Parts<'a> {
    pub level: &'a MeshData,
    pub meshes: &'a [MeshData],
    pub objects: &'a [Object],
    pub sky: Option<usize>,
    pub atmosphere: Atmosphere,
    pub textures: &'a [TextureData],
    pub materials: &'a [Material],
}

impl SceneData {
    /// What the renderer needs of it.
    pub fn parts(&self) -> Parts<'_> {
        Parts {
            level: &self.level,
            meshes: &self.meshes,
            objects: &self.objects,
            sky: self.sky,
            atmosphere: self.atmosphere,
            textures: &self.textures,
            materials: &self.materials,
        }
    }
}

/// A frame read back: `w` by `h` pixels, top row first, each 0x00RRGGBB
/// in sRGB (as the lobby's canvas holds them), and how long it took.
pub struct Rendered {
    pub px: Vec<u32>,
    /// Drawing and copying it on the GPU, waited for (ms).
    pub render_ms: f64,
    /// Mapping it and unpacking the pixels (ms).
    pub readback_ms: f64,
}

/// Bytes in one row of a `w` pixel wide copy to a buffer: 4 a pixel,
/// padded to wgpu's 256.
pub fn padded_row(w: u32) -> u32 {
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    (w * 4).div_ceil(align) * align
}

/// Rows of 4-byte pixels `padded` bytes apart (BGRA, or RGBA when not
/// `bgra`) as 0x00RRGGBB, `w` by `h`, into `out` (cleared first).
pub fn unpack(bytes: &[u8], w: usize, h: usize, padded: usize, bgra: bool, out: &mut Vec<u32>) {
    out.clear();
    out.reserve(w * h);
    for y in 0..h {
        let Some(row) = bytes.get(y * padded..y * padded + w * 4) else {
            out.resize(w * h, 0);
            return;
        };
        for p in row.as_chunks::<4>().0 {
            // Little-endian BGRA read as one number is already 0xAARRGGBB.
            let v = if bgra {
                u32::from_le_bytes(*p)
            } else {
                u32::from_le_bytes([p[2], p[1], p[0], p[3]])
            };
            out.push(v & 0x00FF_FFFF);
        }
    }
}

/// Per-draw uniforms, each at its own `SLOT` (dynamic offset).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DrawUniforms {
    mvp: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    camera: [f32; 4],
    /// w: how the mesh is lit (`OBJECT`, `BAKED`, `UNLIT`).
    params: [f32; 4],
    /// Objects: the level's baked light where they stand; w = 1 when set.
    light: [f32; 4],
    /// Fog: its colour (linear) and density, and the distances it starts
    /// at and is thickest from (z: 1 when it thickens with the square).
    fog: [f32; 4],
    fog_range: [f32; 4],
}

/// How a mesh is lit (the uniform's `params.w`).
const OBJECT: f32 = 0.0;
const BAKED: f32 = 1.0;
const UNLIT: f32 = 2.0;

/// Fog as the shader takes it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Fog {
    /// Linear colour, and density.
    color: [f32; 4],
    /// Where it starts, and where it's thickest.
    range: [f32; 2],
    /// It thickens with the square of the distance.
    squared: bool,
}

/// A tag's (gamma space) colour as the shader takes it.
fn linear([r, g, b]: [f32; 3]) -> [f32; 3] {
    [r, g, b].map(|c| c.max(0.0).powf(2.2))
}

/// The world's fog, the sky's, and the clear colour (linear), from the
/// sky's tag (h2viewer's `set_air`).
fn air(a: &Atmosphere) -> (Fog, Fog, [f32; 3]) {
    let fog = a.fog.map_or(Fog::default(), |f| {
        let [r, g, b] = linear(f.color);
        Fog {
            color: [r, g, b, f.max_density.min(1.0)],
            range: [f.start, f.opaque],
            squared: false,
        }
    });
    let sky = a.sky_fog.map_or(Fog::default(), |(c, density)| {
        let [r, g, b] = linear(c);
        Fog {
            color: [r, g, b, density.min(1.0)],
            range: [0.0; 2],
            squared: false,
        }
    });
    let clear = a.clear_color.map_or(DEFAULT_CLEAR, linear);
    (fog, sky, clear)
}

/// The fragment shader's per-material constants.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MaterialParams {
    illum: [f32; 4],
    tint: [f32; 4],
    mode: [f32; 4],
}

impl MaterialParams {
    fn new(m: &Material) -> MaterialParams {
        let blend = match m.blend {
            Blend::Opaque => 0.0,
            Blend::AlphaTest => 1.0,
            Blend::Alpha => 2.0,
            Blend::Additive => 3.0,
        };
        let aux = match m.aux_kind {
            AuxKind::None => 0.0,
            AuxKind::Illum => 1.0,
            AuxKind::Mask => 2.0,
            AuxKind::ChangeColor => 3.0,
            AuxKind::Emblem => 4.0,
            AuxKind::Multiply => 5.0,
            AuxKind::Multiply2x => 6.0,
        };
        let [r, g, b] = m.illum_color;
        let [tr, tg, tb] = m.tint;
        MaterialParams {
            illum: [r, g, b, 0.0],
            tint: [tr, tg, tb, m.opacity],
            mode: [blend, aux, 0.0, 0.0],
        }
    }
}

/// h2viewer's mesh shader (menu-preview), trimmed to what the menu scene
/// uses: the lightmaps' doubling, the second map's kinds (glow, mask,
/// change colour, emblem, the clouds' multiplied maps), linear and squared
/// fog. Change colour has no armour colours here, and an emblem surface
/// shows nothing (there is no player).
const MESH_SHADER: &str = r#"
struct U {
    mvp: mat4x4<f32>,
    model: mat4x4<f32>,
    camera: vec4<f32>,
    // w: 0 an object, 1 level geometry (baked light), 2 unlit (the sky).
    params: vec4<f32>,
    light: vec4<f32>,
    // Colour and density; x: where it starts, y: where it's thickest,
    // z: 1 when it thickens with the square of the distance.
    fog: vec4<f32>,
    fog_range: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: U;

// Per-material constants: glow colour, tint, and how the surface blends.
struct M {
    illum: vec4<f32>,
    tint: vec4<f32>,
    // x: 0 opaque, 1 alpha tested, 2 alpha blended, 3 additive.
    // y: aux texture is 0 unused, 1 a glow map, 2 an opacity mask,
    //    3 a change-colour map, 4 an emblem, 5 a colour map multiplying
    //    the first, 6 the same doubled.
    mode: vec4<f32>,
};
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;
@group(1) @binding(2) var lightmap: texture_2d<f32>;
@group(1) @binding(3) var lsamp: sampler;
@group(1) @binding(4) var aux: texture_2d<f32>;
@group(1) @binding(5) var<uniform> m: M;

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) lmuv: vec2<f32>,
    @location(4) light: vec4<f32>,
};

@vertex
fn vs(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) lmuv: vec2<f32>,
    @location(4) light: vec4<f32>,
) -> Out {
    var o: Out;
    o.clip = u.mvp * vec4<f32>(position, 1.0);
    o.world = (u.model * vec4<f32>(position, 1.0)).xyz;
    o.normal = (u.model * vec4<f32>(normal, 0.0)).xyz;
    o.uv = uv;
    o.lmuv = lmuv;
    o.light = light;
    return o;
}

// Baked light is stored at half brightness so it can light surfaces up to 2x.
// Halo 2 applied that doubling in gamma space; 2^2.2 is the same in linear.
const LIGHTMAP_SCALE: f32 = 4.59;

@fragment
fn fs(i: Out) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, i.uv);
    let a = textureSample(aux, samp, i.uv);
    let baked = textureSample(lightmap, lsamp, i.lmuv).rgb;
    var alpha = c.a;
    if (m.mode.y > 1.5 && m.mode.y < 2.5) {
        alpha = min(a.r, a.a);
    }
    if (m.mode.y > 4.5 && m.mode.y < 5.5) {
        alpha *= a.a;
    }
    if (m.mode.y > 5.5) {
        // Added clouds: their pass adds the maps' colour alone.
        alpha = 1.0;
    }
    alpha *= m.tint.a;
    // An emblem shows a player's picture; the menus have no player.
    let emblem = m.mode.y > 3.5 && m.mode.y < 4.5;
    if (emblem || (m.mode.x > 0.5 && m.mode.x < 1.5 && alpha < 0.5) || (m.mode.x > 1.5 && alpha < 0.02)) {
        discard;
    }
    // Tag colours are in gamma space.
    var albedo = c.rgb * pow(m.tint.rgb, vec3<f32>(2.2));
    if (m.mode.y > 4.5) {
        // Clouds: a second colour map multiplies the first; the added ones
        // double the product (in gamma space, as the lightmaps do), up to
        // white.
        albedo *= a.rgb;
        if (m.mode.y > 5.5) {
            albedo = min(albedo * LIGHTMAP_SCALE, vec3<f32>(1.0));
        }
    }
    var light: vec3<f32>;
    if (u.params.w > 1.5 || m.mode.x > 2.5 || m.mode.y > 4.5) {
        // Skies, additive glows and clouds carry their own light.
        light = vec3<f32>(1.0);
    } else if (u.params.w > 0.5) {
        // Level geometry: its lightmap page, or per-vertex colour.
        // Lightmap pages are decoded as sRGB; vertex colours arrive in gamma space.
        let colour = pow(i.light.rgb, vec3<f32>(2.2));
        light = colour * mix(vec3<f32>(1.0), baked, i.light.a) * LIGHTMAP_SCALE;
    } else if (u.light.a > 0.5) {
        // Objects: the level's light where they stand, brighter on top.
        // A little fill stands in for the reflections that keep Halo 2's
        // objects readable in dim places.
        let n = normalize(i.normal);
        let base = pow(u.light.rgb, vec3<f32>(2.2)) * LIGHTMAP_SCALE;
        light = base * mix(0.7, 1.3, n.z * 0.5 + 0.5) + vec3<f32>(0.15);
    } else {
        // Objects off the level: sky-from-above / ground-bounce ambient plus a soft sun.
        let n = normalize(i.normal);
        let sun = normalize(vec3<f32>(0.4, -0.3, 0.85));
        let ambient = mix(vec3<f32>(0.32, 0.30, 0.28), vec3<f32>(0.55, 0.60, 0.68), n.z * 0.5 + 0.5);
        light = ambient + vec3<f32>(0.75, 0.72, 0.65) * max(dot(n, sun), 0.0);
    }
    var colour = albedo * light;
    if (m.mode.y > 0.5 && m.mode.y < 1.5) {
        colour += a.rgb * pow(m.illum.rgb, vec3<f32>(2.2));
    }
    // None nearer than where it starts, thickening to its density by where
    // it's thickest.
    let range = max(u.fog_range.y - u.fog_range.x, 0.001);
    var fog = clamp((distance(i.world, u.camera.xyz) - u.fog_range.x) / range, 0.0, 1.0) * u.fog.a;
    if (u.fog_range.z > 0.5) {
        fog = fog * fog;
    }
    if (m.mode.x > 2.5) {
        // Additive: fade out rather than towards the fog colour.
        return vec4<f32>(colour * alpha * (1.0 - fog), 1.0);
    }
    return vec4<f32>(mix(colour, u.fog.rgb, fog), alpha);
}
"#;

/// The veil: one triangle over the whole frame, each pixel of the scene
/// read and graded (`Veil::apply`) on sRGB-encoded values, then decoded
/// for the sRGB target.
const VEIL_SHADER: &str = r#"
struct V {
    // The multiply, and w: the colour kept.
    multiply: vec4<f32>,
    // The add, and w: the gain.
    add: vec4<f32>,
};
@group(0) @binding(0) var<uniform> v: V;
@group(0) @binding(1) var scene: texture_2d<f32>;

fn encode(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let hi = 1.055 * pow(max(x, vec3<f32>(0.0031308)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, x * 12.92, x <= vec3<f32>(0.0031308));
}

fn decode(c: vec3<f32>) -> vec3<f32> {
    let x = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let hi = pow((max(x, vec3<f32>(0.04045)) + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, x / 12.92, x <= vec3<f32>(0.04045));
}

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32((i << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(i & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

@fragment
fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(scene, vec2<i32>(at.xy), 0).rgb;
    let grey = encode(vec3<f32>(dot(c, vec3<f32>(0.2126, 0.7152, 0.0722))));
    let kept = grey + (encode(c) - grey) * v.multiply.w;
    let lit = clamp(kept * v.add.w, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(decode(lit * v.multiply.rgb + v.add.rgb), 1.0);
}
"#;

/// A mesh on the GPU, and each batch's material bind group, index range
/// and blending.
struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    batches: Vec<(usize, std::ops::Range<u32>, Blend)>,
}

/// The scene on the GPU.
struct GpuScene {
    level: Option<GpuMesh>,
    meshes: Vec<Option<GpuMesh>>,
    /// One bind group per (material, lightmap page) the meshes use.
    materials: Vec<wgpu::BindGroup>,
    /// Objects: mesh, transform, light.
    objects: Vec<(usize, Mat4, Option<[f32; 3]>)>,
    sky: Option<usize>,
    fog: Fog,
    sky_fog: Fog,
    clear: [f32; 3],
}

/// The scene drawn into (`scene`, with its depth), the frame the veil
/// makes of it (`color`), and where that is copied for reading.
struct Target {
    w: u32,
    h: u32,
    scene: wgpu::TextureView,
    depth: wgpu::TextureView,
    color: wgpu::Texture,
    view: wgpu::TextureView,
    /// The veil's uniform and the scene, for its pass.
    veil: wgpu::BindGroup,
    readback: wgpu::Buffer,
}

/// The menu scene's renderer: a wgpu device of its own, with no window.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    info: wgpu::AdapterInfo,
    /// The frame's format: BGRA in sRGB, else RGBA in sRGB.
    format: wgpu::TextureFormat,
    /// Set by the device's error and lost callbacks.
    failed: Arc<Mutex<Option<String>>>,
    opaque: wgpu::RenderPipeline,
    alpha: wgpu::RenderPipeline,
    additive: wgpu::RenderPipeline,
    sky: wgpu::RenderPipeline,
    sky_alpha: wgpu::RenderPipeline,
    veil_pipeline: wgpu::RenderPipeline,
    globals_layout: wgpu::BindGroupLayout,
    material_layout: wgpu::BindGroupLayout,
    veil_layout: wgpu::BindGroupLayout,
    repeat: wgpu::Sampler,
    clamp: wgpu::Sampler,
    veil_buffer: wgpu::Buffer,
    /// Draw uniforms, a `SLOT` each, and their bind group.
    uniforms: Option<(wgpu::Buffer, wgpu::BindGroup, u64)>,
    staging: Vec<u8>,
    scene: Option<GpuScene>,
    target: Option<Target>,
    /// The last frame's pixels, reused.
    unpacked: Vec<u32>,
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn uniform_entry(binding: u32, dynamic: bool, size: Option<u64>) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: dynamic,
            min_binding_size: size.and_then(wgpu::BufferSize::new),
        },
        count: None,
    }
}

/// Whether an adapter of `kind` is refused: a software one (lavapipe,
/// WARP, llvmpipe, the Microsoft Basic Render Driver of a remote desktop
/// or a virtual machine) unless `allow_cpu`, as drawing the scene on it
/// would cost the CPU more than the still background.
fn refused(kind: wgpu::DeviceType, allow_cpu: bool) -> bool {
    kind == wgpu::DeviceType::Cpu && !allow_cpu
}

impl Renderer {
    /// A device on this PC's GPU, from the instance's backends (so
    /// `WGPU_BACKEND` is honoured), preferring the low-power one. A
    /// software adapter (lavapipe, WARP, llvmpipe) is refused unless
    /// `allow_cpu`: it would cost the CPU more than the still background.
    pub fn new(allow_cpu: bool, veil: Veil) -> Result<Renderer, String> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .map_err(|e| format!("no GPU adapter ({e})"))?;
        let info = adapter.get_info();
        if refused(info.device_type, allow_cpu) {
            return Err(format!(
                "only a software adapter ({}, {:?}), refused",
                info.name, info.backend
            ));
        }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("menu scene"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| format!("{} ({:?}): no device ({e})", info.name, info.backend))?;
        // wgpu's own handler panics, which would take the lobby (and its
        // party) down: these note the failure for the next frame instead.
        let failed = Arc::new(Mutex::new(None::<String>));
        let noted = failed.clone();
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            if let Ok(mut f) = noted.lock() {
                f.get_or_insert_with(|| format!("GPU error: {e}"));
            }
        }));
        let noted = failed.clone();
        device.set_device_lost_callback(move |reason, message| {
            if let Ok(mut f) = noted.lock() {
                f.get_or_insert_with(|| format!("GPU device lost ({reason:?}): {message}"));
            }
        });
        let bgra = wgpu::TextureFormat::Bgra8UnormSrgb;
        let usable = |f| {
            adapter
                .get_texture_format_features(f)
                .allowed_usages
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        };
        let format = if usable(bgra) {
            bgra
        } else {
            wgpu::TextureFormat::Rgba8UnormSrgb
        };

        let draw_size = std::mem::size_of::<DrawUniforms>() as u64;
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("draw"),
            entries: &[uniform_entry(0, true, Some(draw_size))],
        });
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material"),
            entries: &[
                texture_entry(0),
                sampler_entry(1),
                texture_entry(2),
                sampler_entry(3),
                texture_entry(4),
                uniform_entry(5, false, None),
            ],
        });
        let veil_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("veil"),
            entries: &[
                uniform_entry(0, false, None),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let repeat = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("repeat"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let clamp = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("clamp"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let mesh_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh"),
            bind_group_layouts: &[Some(&globals_layout), Some(&material_layout)],
            immediate_size: 0,
        });
        let module = |label, src: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(src.into()),
            })
        };
        let mesh_shader = module("mesh", MESH_SHADER);
        let veil_shader = module("veil", VEIL_SHADER);

        let opaque_target = [Some(wgpu::ColorTargetState::from(SCENE_FORMAT))];
        let alpha_target = [Some(wgpu::ColorTargetState {
            format: SCENE_FORMAT,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let additive_target = [Some(wgpu::ColorTargetState {
            format: SCENE_FORMAT,
            blend: Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent::OVER,
            }),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let pipeline = |label, cull, depth_write, targets: &[Option<wgpu::ColorTargetState>]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&mesh_layout),
                vertex: wgpu::VertexState {
                    module: &mesh_shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: Vertex::SIZE as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x3,
                            1 => Float32x3,
                            2 => Float32x2,
                            3 => Float32x2,
                            4 => Float32x4
                        ],
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: cull,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &mesh_shader,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let opaque = pipeline("opaque", Some(wgpu::Face::Back), true, &opaque_target);
        // Skies are seen from inside, and layer their see-through parts
        // with depth so nearer layers hide farther ones.
        let sky = pipeline("sky", None, true, &opaque_target);
        let alpha = pipeline("alpha", None, false, &alpha_target);
        let sky_alpha = pipeline("sky alpha", None, true, &alpha_target);
        let additive = pipeline("additive", None, false, &additive_target);

        let veil_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("veil"),
            bind_group_layouts: &[Some(&veil_layout)],
            immediate_size: 0,
        });
        let veil_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("veil"),
            layout: Some(&veil_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &veil_shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &veil_shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState::from(format))],
            }),
            multiview_mask: None,
            cache: None,
        });
        let veil_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("veil"),
            contents: bytemuck::bytes_of(&veil.uniform()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let r = Renderer {
            device,
            queue,
            info,
            format,
            failed,
            opaque,
            alpha,
            additive,
            sky,
            sky_alpha,
            veil_pipeline,
            globals_layout,
            material_layout,
            veil_layout,
            repeat,
            clamp,
            veil_buffer,
            uniforms: None,
            staging: Vec::new(),
            scene: None,
            target: None,
            unpacked: Vec::new(),
        };
        r.check()?;
        Ok(r)
    }

    /// The adapter, for the log: its name and backend.
    pub fn adapter(&self) -> String {
        let i = &self.info;
        let kind = match i.device_type {
            wgpu::DeviceType::IntegratedGpu => "integrated",
            wgpu::DeviceType::DiscreteGpu => "discrete",
            wgpu::DeviceType::VirtualGpu => "virtual",
            wgpu::DeviceType::Cpu => "software",
            wgpu::DeviceType::Other => "other",
        };
        format!("{} ({kind}, {:?})", i.name, i.backend)
    }

    /// The veil for the frames from now on.
    pub fn set_veil(&mut self, veil: Veil) {
        self.queue
            .write_buffer(&self.veil_buffer, 0, bytemuck::bytes_of(&veil.uniform()));
    }

    /// An error the device reported since, if any.
    fn check(&self) -> Result<(), String> {
        match self.failed.lock() {
            Ok(f) => f.clone().map_or(Ok(()), Err),
            Err(_) => Err("GPU error handler poisoned".into()),
        }
    }

    /// Put `scene` on the GPU, replacing what was there.
    pub fn upload(&mut self, scene: Parts) -> Result<(), String> {
        let max = self.device.limits().max_texture_dimension_2d;
        let views: Vec<wgpu::TextureView> = scene
            .textures
            .iter()
            .map(|t| self.texture(t, max))
            .collect();
        let fallback = self.texture(&TextureData::fallback(), max);
        let mut keys: Vec<(usize, usize)> = Vec::new();
        let mut index: HashMap<(usize, usize), usize> = HashMap::new();
        let mut mesh = |m: &MeshData| -> Option<GpuMesh> {
            if m.indices.is_empty() || m.vertices.is_empty() {
                return None;
            }
            let batches = m
                .batches
                .iter()
                .map(|b| {
                    let key = (b.material, b.lightmap);
                    let k = *index.entry(key).or_insert_with(|| {
                        keys.push(key);
                        keys.len() - 1
                    });
                    let blend = scene
                        .materials
                        .get(b.material)
                        .map_or(Blend::Opaque, |m| m.blend);
                    (k, b.first_index..b.first_index + b.index_count, blend)
                })
                .collect();
            Some(GpuMesh {
                vertices: self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("vertices"),
                        contents: &m.vertex_bytes(),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
                indices: self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("indices"),
                        contents: &m.index_bytes(),
                        usage: wgpu::BufferUsages::INDEX,
                    }),
                batches,
            })
        };
        let level = mesh(scene.level);
        let meshes: Vec<Option<GpuMesh>> = scene.meshes.iter().map(&mut mesh).collect();
        let view = |i: usize| views.get(i).unwrap_or(&fallback);
        let materials = keys
            .iter()
            .map(|&(material, page)| {
                let mat = scene.materials.get(material).cloned().unwrap_or_default();
                let params = MaterialParams::new(&mat);
                let buffer = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: None,
                        contents: bytemuck::bytes_of(&params),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &self.material_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(view(mat.texture)),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&self.repeat),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(view(page)),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::Sampler(&self.clamp),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::TextureView(view(mat.aux)),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: buffer.as_entire_binding(),
                        },
                    ],
                })
            })
            .collect();
        let objects = scene
            .objects
            .iter()
            .map(|o| (o.mesh, o.transform, o.light))
            .collect::<Vec<_>>();
        // A slot for the sky, the level and each object.
        let slots = 2 + objects.len() as u64;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("draw uniforms"),
            size: SLOT * slots,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("draw"),
            layout: &self.globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(std::mem::size_of::<DrawUniforms>() as u64),
                }),
            }],
        });
        self.uniforms = Some((buffer, group, slots));
        let (fog, sky_fog, clear) = air(&scene.atmosphere);
        self.scene = Some(GpuScene {
            level,
            meshes,
            materials,
            objects,
            sky: scene.sky,
            fog,
            sky_fog,
            clear,
        });
        self.check()
    }

    /// A texture with its mips, skipping the largest levels the GPU can't
    /// hold.
    fn texture(&self, t: &TextureData, max: u32) -> wgpu::TextureView {
        let first = (0..t.mips.len())
            .find(|&l| {
                let (w, h) = t.size(l);
                w <= max && h <= max
            })
            .unwrap_or(t.mips.len().saturating_sub(1));
        let (w, h) = t.size(first);
        let levels = (t.mips.len() - first).max(1) as u32;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: w.min(max),
                height: h.min(max),
                depth_or_array_layers: 1,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (k, data) in t.mips.iter().skip(first).enumerate() {
            let (w, h) = t.size(first + k);
            if data.len() != (w as usize) * (h as usize) * 4 || w > max || h > max {
                continue;
            }
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: k as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * 4),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        }
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// The frame, its depth and its read-back buffer at `w` by `h`.
    fn target(&mut self, w: u32, h: u32) -> Result<&Target, String> {
        if self.target.as_ref().is_some_and(|t| (t.w, t.h) == (w, h)) {
            return self.target.as_ref().ok_or_else(String::new);
        }
        let max = self.device.limits().max_texture_dimension_2d;
        if w == 0 || h == 0 || w > max || h > max {
            return Err(format!("{w}x{h} is more than this GPU draws ({max})"));
        }
        self.target = None;
        let size = wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        };
        let scene = self
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("menu scene"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: SCENE_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let color = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("menu scene veiled"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = self
            .device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("depth"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read back"),
            size: u64::from(padded_row(w)) * u64::from(h),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let veil = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("veil"),
            layout: &self.veil_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.veil_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&scene),
                },
            ],
        });
        self.target = Some(Target {
            w,
            h,
            scene,
            depth,
            color,
            view,
            veil,
            readback,
        });
        self.target.as_ref().ok_or_else(String::new)
    }

    /// The uniforms of a draw, appended to `staging`.
    fn slot(
        &mut self,
        mvp: Mat4,
        model: Mat4,
        eye: Vec3,
        fog: Fog,
        shading: f32,
        light: Option<[f32; 3]>,
    ) {
        let u = DrawUniforms {
            mvp: (mvp * model).to_cols_array_2d(),
            model: model.to_cols_array_2d(),
            camera: eye.extend(1.0).into(),
            params: [0.0, 0.0, 0.0, shading],
            light: light.map_or([0.0; 4], |[r, g, b]| [r, g, b, 1.0]),
            fog: fog.color,
            fog_range: [
                fog.range[0],
                fog.range[1],
                f32::from(u8::from(fog.squared)),
                0.0,
            ],
        };
        let at = self.staging.len();
        self.staging.extend_from_slice(bytemuck::bytes_of(&u));
        self.staging.resize(at + SLOT as usize, 0);
    }

    /// Draw the uploaded scene seen from `eye` into a `w` by `h` frame,
    /// veil it, and read it back.
    pub fn render(&mut self, w: u32, h: u32, eye: Eye) -> Result<Rendered, String> {
        self.check()?;
        let started = Instant::now();
        let Some(scene) = self.scene.take() else {
            return Err("no scene uploaded".into());
        };
        let out = self.draw(&scene, w, h, eye);
        self.scene = Some(scene);
        let render_ms = started.elapsed().as_secs_f64() * 1000.0;
        out?;
        let read = Instant::now();
        let px = self.read_back()?;
        Ok(Rendered {
            px,
            render_ms,
            readback_ms: read.elapsed().as_secs_f64() * 1000.0,
        })
    }

    fn draw(&mut self, scene: &GpuScene, w: u32, h: u32, eye: Eye) -> Result<(), String> {
        self.target(w, h)?;
        let (sky_proj, world_proj) = eye.view_projs(w as f32 / h.max(1) as f32);
        self.staging.clear();
        self.slot(sky_proj, Mat4::IDENTITY, eye.at, scene.sky_fog, UNLIT, None);
        self.slot(world_proj, Mat4::IDENTITY, eye.at, scene.fog, BAKED, None);
        for &(_, model, light) in &scene.objects {
            self.slot(world_proj, model, eye.at, scene.fog, OBJECT, light);
        }
        let (Some((buffer, group, _)), Some(t)) = (&self.uniforms, &self.target) else {
            return Err("no uniforms".into());
        };
        self.queue.write_buffer(buffer, 0, &self.staging);
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let [r, g, b] = scene.clear.map(f64::from);
        let depth = || wgpu::RenderPassDepthStencilAttachment {
            view: &t.depth,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(0.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        };
        let color = |load| wgpu::RenderPassColorAttachment {
            view: &t.scene,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            },
        };
        let slot = |k: u64| (k * SLOT) as u32;
        // The sky behind everything, with its own depth.
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sky"),
                color_attachments: &[Some(color(wgpu::LoadOp::Clear(wgpu::Color {
                    r,
                    g,
                    b,
                    a: 1.0,
                })))],
                depth_stencil_attachment: Some(depth()),
                ..Default::default()
            });
            let sky = scene.sky.and_then(|s| scene.meshes.get(s)?.as_ref());
            if let Some(m) = sky {
                let meshes = [(m, slot(0))];
                self.draw_meshes(
                    &mut pass,
                    scene,
                    group,
                    &meshes,
                    (&self.sky, &self.sky_alpha),
                );
            }
        }
        // The level and its scenery: opaque, then alpha blended, then
        // additive.
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world"),
                color_attachments: &[Some(color(wgpu::LoadOp::Load))],
                depth_stencil_attachment: Some(depth()),
                ..Default::default()
            });
            let mut meshes = Vec::new();
            if let Some(level) = &scene.level {
                meshes.push((level, slot(1)));
            }
            for (k, (mesh, _, _)) in scene.objects.iter().enumerate() {
                if let Some(Some(m)) = scene.meshes.get(*mesh) {
                    meshes.push((m, slot(2 + k as u64)));
                }
            }
            self.draw_meshes(
                &mut pass,
                scene,
                group,
                &meshes,
                (&self.opaque, &self.alpha),
            );
        }
        // The veil over it all, into the frame read back.
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("veil"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.veil_pipeline);
            pass.set_bind_group(0, &t.veil, &[]);
            pass.draw(0..3, 0..1);
        }
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &t.color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &t.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row(w)),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let index = self.queue.submit([enc.finish()]);
        // A frame the GPU refused says why there.
        self.check()?;
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(index),
                timeout: Some(GPU_WAIT),
            })
            .map_err(|e| format!("the GPU didn't finish the frame: {e}"))?;
        self.check()
    }

    /// Draw meshes (each with its uniform offset): their opaque surfaces,
    /// then the alpha blended ones, then the additive ones, each with its
    /// pipeline.
    fn draw_meshes(
        &self,
        pass: &mut wgpu::RenderPass,
        scene: &GpuScene,
        group: &wgpu::BindGroup,
        meshes: &[(&GpuMesh, u32)],
        (opaque, alpha): (&wgpu::RenderPipeline, &wgpu::RenderPipeline),
    ) {
        let passes = [
            (opaque, [Blend::Opaque, Blend::AlphaTest]),
            (alpha, [Blend::Alpha, Blend::Alpha]),
            (&self.additive, [Blend::Additive, Blend::Additive]),
        ];
        for (pipeline, blends) in passes {
            pass.set_pipeline(pipeline);
            for &(m, offset) in meshes {
                pass.set_bind_group(0, group, &[offset]);
                pass.set_vertex_buffer(0, m.vertices.slice(..));
                pass.set_index_buffer(m.indices.slice(..), wgpu::IndexFormat::Uint32);
                for (material, range, blend) in &m.batches {
                    if blends.contains(blend) {
                        pass.set_bind_group(1, &scene.materials[*material], &[]);
                        pass.draw_indexed(range.clone(), 0, 0..1);
                    }
                }
            }
        }
    }

    /// The frame just drawn, as pixels.
    fn read_back(&mut self) -> Result<Vec<u32>, String> {
        let t = self.target.as_ref().ok_or("no frame")?;
        let slice = t.readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(GPU_WAIT),
            })
            .map_err(|e| format!("the frame couldn't be read back: {e}"))?;
        match rx.try_recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(format!("the frame couldn't be read back: {e}")),
            Err(_) => return Err("the frame wasn't read back in time".into()),
        }
        let bgra = self.format == wgpu::TextureFormat::Bgra8UnormSrgb;
        {
            let bytes = slice
                .get_mapped_range()
                .map_err(|e| format!("the frame couldn't be read back: {e}"))?;
            let (w, h) = (t.w as usize, t.h as usize);
            unpack(
                &bytes,
                w,
                h,
                padded_row(t.w) as usize,
                bgra,
                &mut self.unpacked,
            );
        }
        t.readback.unmap();
        self.check()?;
        Ok(std::mem::take(&mut self.unpacked))
    }

    /// Give back a frame's pixels to be drawn into again.
    pub fn recycle(&mut self, px: Vec<u32>) {
        self.unpacked = px;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Batch;

    #[test]
    fn rows_are_padded_to_256_bytes() {
        assert_eq!(padded_row(1), 256);
        assert_eq!(padded_row(64), 256);
        assert_eq!(padded_row(65), 512);
        assert_eq!(padded_row(1920), 7680);
        assert_eq!(padded_row(1366), 5632);
        // Every row holds its pixels.
        for w in [1, 3, 63, 64, 65, 1279, 1280, 1366, 2561] {
            assert!(padded_row(w) >= w * 4 && padded_row(w).is_multiple_of(256));
        }
    }

    #[test]
    fn pixels_unpack_past_the_padding() {
        // 3 by 2 pixels, rows 16 bytes apart (4 bytes of padding each).
        let mut bytes = vec![0xEEu8; 32];
        let px = |b: u8, g: u8, r: u8| [b, g, r, 0xFF];
        for (k, p) in [
            px(1, 2, 3),
            px(4, 5, 6),
            px(7, 8, 9),
            px(10, 11, 12),
            px(13, 14, 15),
            px(16, 17, 18),
        ]
        .iter()
        .enumerate()
        {
            let at = (k / 3) * 16 + (k % 3) * 4;
            bytes[at..at + 4].copy_from_slice(p);
        }
        let mut out = vec![7; 99];
        unpack(&bytes, 3, 2, 16, true, &mut out);
        assert_eq!(
            out,
            vec![0x030201, 0x060504, 0x090807, 0x0C0B0A, 0x0F0E0D, 0x121110]
        );
        // RGBA rows swap red and blue; alpha never shows.
        unpack(&bytes, 3, 2, 16, false, &mut out);
        assert_eq!(out[0], 0x010203);
        assert!(out.iter().all(|p| p >> 24 == 0));
        // Too few bytes: the rest is black, not a panic.
        unpack(&bytes[..20], 3, 2, 16, true, &mut out);
        assert_eq!(out.len(), 6);
        assert_eq!(out[3..], [0, 0, 0]);
    }

    #[test]
    fn srgb_round_trips() {
        for c in 0..=255u8 {
            assert_eq!(linear_to_srgb(srgb_to_linear(c)), c);
        }
        assert_eq!(srgb_to_linear(0), 0.0);
        assert!((srgb_to_linear(255) - 1.0).abs() < 1e-6);
        assert!((srgb_to_linear(188) - 0.5).abs() < 0.01);
    }

    #[test]
    fn the_veil_greys_brightens_multiplies_then_adds_on_srgb_values() {
        // None leaves every colour.
        for c in [[0, 0, 0], [10, 120, 250], [128, 64, 32], [255, 255, 255]] {
            assert_eq!(Veil::NONE.apply_srgb(c), c);
        }
        // Keeping all the colour, with no gain: black takes the added
        // colour alone, white the multiply plus it, each channel on its
        // sRGB value.
        let v = Veil::navy([100, 120, 200], 0.25, 1.0, 1.0);
        let navy = [100u8, 120, 200].map(|c| f32::from(c) / 255.0);
        assert_eq!(v.multiply, navy);
        assert_eq!(v.add, navy.map(|c| c * 0.25));
        assert_eq!(v.apply([0.0; 3]), v.add.map(srgb_decode));
        let white = v.apply([1.0; 3]);
        for c in 0..3 {
            assert!((srgb_encode(white[c]) - navy[c] * 1.25).abs() < 1e-5);
        }
        // In sRGB bytes: 128 x navy + navy/4.
        let grey = v.apply_srgb([128, 128, 128]);
        let want = [100.0f32, 120.0, 200.0].map(|n| (128.0 / 255.0 * n + n * 0.25).round() as u8);
        assert_eq!(grey, want);
        // Nothing past white.
        assert_eq!(
            Veil::navy([255; 3], 4.0, 1.0, 1.0).apply_srgb([255; 3]),
            [255; 3]
        );
        // The gain brightens, and what passes white is clipped there.
        let lit = Veil::navy([255; 3], 0.0, 1.0, 2.0);
        assert_eq!(lit.apply_srgb([60, 60, 60]), [120; 3]);
        assert_eq!(lit.apply_srgb([200, 100, 10]), [255, 200, 20]);
        // Keeping no colour, every pixel is its grey (its luminance,
        // encoded) times the navy: a brown and a purple of the same
        // brightness come out the same.
        let mono = Veil::navy([100, 120, 200], 0.0, 0.0, 1.0);
        let brown = [0.30f32, 0.19, 0.08];
        let y: f32 = (0..3).map(|c| brown[c] * LUMA[c]).sum();
        let purple = [y + 0.1 * LUMA[1] / LUMA[0], y - 0.1, y + 0.0];
        let py: f32 = (0..3).map(|c| purple[c] * LUMA[c]).sum();
        assert!((py - y).abs() < 1e-6);
        let out = mono.apply(brown);
        let other = mono.apply(purple);
        for c in 0..3 {
            assert!((srgb_encode(out[c]) - srgb_encode(y) * navy[c]).abs() < 1e-5);
            assert!((out[c] - other[c]).abs() < 1e-5);
        }
        // Half the colour: halfway between the grey and the colour, on
        // their sRGB values.
        let half = Veil::navy([255; 3], 0.0, 0.5, 1.0).apply(brown);
        for c in 0..3 {
            let want = (srgb_encode(y) + srgb_encode(brown[c])) / 2.0;
            assert!((srgb_encode(half[c]) - want).abs() < 1e-5);
        }
    }

    #[test]
    fn the_menus_veil_clips_white_at_the_navy_and_keeps_its_hue_at_every_brightness() {
        let m = Veil::menu();
        // The screenshot's brightest: white (and anything past it once
        // brightened) is the navy; black stays black.
        assert_eq!(m.apply_srgb([255; 3]), MENU_NAVY);
        assert_eq!(m.apply_srgb([200, 210, 190]), MENU_NAVY);
        assert_eq!(m.apply_srgb([0; 3]), [0; 3]);
        // Below the clip, every pixel is the navy scaled (in sRGB bytes),
        // so its hue is the same as the screenshot's at every brightness.
        for v in [8u8, 20, 40, 60, 90, 110] {
            let out = m.apply_srgb([v; 3]);
            let scale = (f32::from(v) / 255.0 * MENU_GAIN).min(1.0);
            for c in 0..3 {
                let want = f32::from(MENU_NAVY[c]) * scale;
                assert!(
                    (f32::from(out[c]) - want).abs() <= 1.0,
                    "{v}: {out:?}, want {want} in {c}"
                );
            }
            assert!(out[2] > out[1] && out[1] >= out[0], "{v}: {out:?}");
        }
        // A brown sky the same navy as a grey of its brightness.
        let sky = m.apply_srgb([150, 120, 80]);
        let y: f32 = (0..3)
            .map(|c| [150u8, 120, 80].map(srgb_to_linear)[c] * LUMA[c])
            .sum();
        let like = m.apply_srgb([linear_to_srgb(y); 3]);
        assert!(
            (0..3).all(|c| sky[c].abs_diff(like[c]) <= 1),
            "{sky:?} {like:?}"
        );
    }

    #[test]
    fn veils_read_from_four_five_or_six_numbers() {
        assert_eq!(
            Veil::parse("100, 120,200,0.25"),
            Some(Veil::navy(
                [100, 120, 200],
                0.25,
                MENU_SATURATION,
                MENU_GAIN
            ))
        );
        assert_eq!(
            Veil::parse("100,120,200,0.25,0.5"),
            Some(Veil::navy([100, 120, 200], 0.25, 0.5, MENU_GAIN))
        );
        assert_eq!(
            Veil::parse("44,55,88,0,0,3"),
            Some(Veil::navy([44, 55, 88], 0.0, 0.0, 3.0))
        );
        assert_eq!(
            Veil::parse("0,0,0,0"),
            Some(Veil::navy([0; 3], 0.0, MENU_SATURATION, MENU_GAIN))
        );
        for bad in [
            "",
            "1,2,3",
            "1,2,3,4,5,6,7",
            "1,2,3,4,1.5",
            "1,2,3,4,1,-2",
            "1,2,3,4,1,inf",
            "256,0,0,1",
            "a,b,c,d",
            "1,2,3,-1",
            "1,2,3,nan",
        ] {
            assert_eq!(Veil::parse(bad), None, "{bad:?}");
        }
        // The menus' is the estimate.
        assert_eq!(
            Veil::menu(),
            Veil::navy(MENU_NAVY, MENU_K, MENU_SATURATION, MENU_GAIN)
        );
    }

    #[test]
    fn a_software_adapter_is_refused_unless_allowed() {
        use wgpu::DeviceType::*;
        assert!(refused(Cpu, false));
        assert!(!refused(Cpu, true));
        for kind in [IntegratedGpu, DiscreteGpu, VirtualGpu, Other] {
            assert!(!refused(kind, false), "{kind:?}");
            assert!(!refused(kind, true), "{kind:?}");
        }
        // And on this machine: refused, or not a software adapter (where
        // lavapipe is the only one, as in the container, this checks
        // `new` itself asks).
        match Renderer::new(false, Veil::NONE) {
            Ok(r) => assert_ne!(r.info.device_type, Cpu, "{}", r.adapter()),
            Err(e) => eprintln!("refused or none: {e}"),
        }
    }

    /// A square of two triangles facing -x (towards a camera at the
    /// origin looking +x), `half` across, `d` away.
    fn square(d: f32, half: f32, light: [f32; 4]) -> MeshData {
        let v = |y: f32, z: f32| Vertex {
            light,
            ..Vertex::new([d, y, z], [-1.0, 0.0, 0.0], [0.5, 0.5])
        };
        MeshData {
            vertices: vec![
                v(half, -half),
                v(-half, -half),
                v(-half, half),
                v(half, half),
            ],
            // Counter-clockwise as seen from the camera: +y is to the
            // left, +z up.
            indices: vec![0, 1, 2, 0, 2, 3],
            batches: vec![Batch {
                material: 1,
                lightmap: 0,
                first_index: 0,
                index_count: 6,
            }],
            baked_lighting: true,
        }
    }

    /// A one-colour texture.
    fn solid(rgb: [u8; 3]) -> TextureData {
        TextureData::new(2, 2, [rgb[0], rgb[1], rgb[2], 255].repeat(4)).unwrap()
    }

    /// The scene renderer on this PC's GPU, or None (the test is skipped)
    /// without one. Lavapipe counts with `H2LOBBY_SCENE=on`.
    fn gpu(veil: Veil) -> Option<Renderer> {
        let cpu = std::env::var("H2LOBBY_SCENE").is_ok_and(|v| v == "on");
        match Renderer::new(cpu, veil) {
            Ok(r) => {
                eprintln!("scene test on {}", r.adapter());
                Some(r)
            }
            Err(e) => {
                eprintln!("skipped: {e}");
                None
            }
        }
    }

    #[test]
    fn a_textured_square_and_the_sky_draw_through_the_real_pipelines() {
        let Some(mut r) = gpu(Veil::NONE) else {
            return;
        };
        // The sky: a wide red square far off, unlit. The level: a small
        // green square in front of it, lit by its vertices at exactly
        // 1 (2.2 power, times the lightmaps' 4.59). Around them the clear
        // colour, a dark grey.
        let unit = (1.0f32 / 4.59).powf(1.0 / 2.2);
        let sky = square(100.0, 60.0, [1.0; 4]);
        let level = square(10.0, 2.0, [unit, unit, unit, 0.0]);
        let mut materials = vec![Material::plain(0), Material::plain(1), Material::plain(2)];
        materials[1].texture = 1;
        let sky_material = 2;
        let mut sky = sky;
        sky.batches[0].material = sky_material;
        let textures = vec![
            TextureData::fallback(),
            solid([0, 200, 0]),
            solid([220, 0, 0]),
        ];
        let atmosphere = Atmosphere {
            fog: None,
            sky_fog: None,
            clear_color: Some([0.2, 0.2, 0.2]),
        };
        let meshes = [sky];
        let parts = Parts {
            level: &level,
            meshes: &meshes,
            objects: &[],
            sky: Some(0),
            atmosphere,
            textures: &textures,
            materials: &materials,
        };
        r.upload(parts).unwrap();
        let eye = Eye {
            at: Vec3::ZERO,
            forward: Vec3::X,
            up: Vec3::Z,
        };
        let (w, h) = (160u32, 90u32);
        let frame = r.render(w, h, eye).unwrap();
        assert_eq!(frame.px.len(), (w * h) as usize);
        let at = |x: u32, y: u32| frame.px[(y * w + x) as usize];
        let rgb = |p: u32| [(p >> 16) as u8, (p >> 8) as u8, p as u8];
        let near = |p: u32, want: [u8; 3]| {
            let got = rgb(p);
            (0..3).all(|c| got[c].abs_diff(want[c]) <= 2)
        };
        // The green square in the middle, as its texture.
        assert!(near(at(80, 45), [0, 200, 0]), "{:06x}", at(80, 45));
        // The red sky around it.
        assert!(near(at(40, 20), [220, 0, 0]), "{:06x}", at(40, 20));
        // The clear colour at the corners, beyond the sky's square: its
        // gamma 0.2 is linear 0.2^2.2, shown as sRGB.
        let grey = linear_to_srgb(0.2f32.powf(2.2));
        assert!(near(at(0, 0), [grey; 3]), "{:06x}", at(0, 0));
        assert!(near(at(w - 1, h - 1), [grey; 3]));
        // The square sits in the middle: its edges are where the lens
        // puts them (2 units across at 10, 70 degrees across 16:9).
        let half = (2.0 / 10.0) / (FOV.to_radians() * 0.5).tan() * (w as f32 / 2.0);
        let edge = (80.0 + half) as u32;
        assert!(near(at(edge - 2, 45), [0, 200, 0]));
        assert!(near(at(edge + 2, 45), [220, 0, 0]));

        // The veil over the same frame: as `Veil::apply` says, within a
        // step of rounding.
        let veil = Veil::navy([90, 110, 170], 0.1, 0.3, 1.6);
        r.set_veil(veil);
        let veiled = r.render(w, h, eye).unwrap();
        for (x, y, was) in [
            (80, 45, [0, 200, 0]),
            (40, 20, [220, 0, 0]),
            (0, 0, [grey; 3]),
        ] {
            let p = veiled.px[(y * w + x) as usize];
            let want = veil.apply_srgb(was);
            assert!(near(p, want), "({x}, {y}): {:06x}, want {want:?}", p);
        }
        // Another size: a new frame, the same picture.
        let small = r.render(64, 36, eye).unwrap();
        assert_eq!(small.px.len(), 64 * 36);
        assert!(near(small.px[18 * 64 + 32], veil.apply_srgb([0, 200, 0])));
    }

    /// The real scene from `H2_MAPS` (Halo 2 Vista's mainmenu.map) at 10,
    /// 40 and 70 seconds, with the menus' veil: never one flat colour,
    /// and navy on the whole. With `H2_SCENE_PNG=<folder>` the frames are
    /// kept there as PNGs (pictures of the game's files: never commit
    /// them). A software adapter is allowed.
    #[test]
    #[ignore = "needs H2_MAPS with Halo 2 Vista's mainmenu.map, and a GPU or lavapipe"]
    fn draws_the_real_main_menu() {
        let dir = std::env::var("H2_MAPS").expect("H2_MAPS");
        let path = std::path::Path::new(&dir).join("mainmenu.map");
        let scene = crate::load(&path, &|l| println!("{l}")).unwrap();
        let mut r = match Renderer::new(true, Veil::menu()) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("skipped: {e}");
                return;
            }
        };
        println!("on {}", r.adapter());
        r.upload(scene.parts()).unwrap();
        let out = std::env::var_os("H2_SCENE_PNG").map(std::path::PathBuf::from);
        let (w, h) = (640u32, 360u32);
        for t in [10.0f32, 40.0, 70.0] {
            let frame = r
                .render(w, h, Eye::flythrough(&scene.flythrough, t))
                .unwrap();
            let px = &frame.px;
            let distinct: std::collections::HashSet<u32> = px.iter().copied().collect();
            let mean = |shift: u32| {
                px.iter()
                    .map(|p| u64::from((p >> shift) & 0xFF))
                    .sum::<u64>()
                    / px.len() as u64
            };
            let (mr, mg, mb) = (mean(16), mean(8), mean(0));
            // The blues' spread: from the darkest twentieth to the
            // brightest.
            let mut blues: Vec<u32> = px.iter().map(|p| p & 0xFF).collect();
            blues.sort_unstable();
            let spread = blues[blues.len() * 19 / 20] - blues[blues.len() / 20];
            println!(
                "{t} s: {} colours, mean ({mr}, {mg}, {mb}), blues spread {spread}, {:.1} ms",
                distinct.len(),
                frame.render_ms
            );
            // The veil keeps no colour, so few distinct ones; but light
            // and dark.
            assert!(distinct.len() > 30 && spread >= 8, "{t} s: nearly flat");
            assert!(mb > mr && mb > 20 && mb < 140, "{t} s: not navy");
            if let Some(dir) = &out {
                std::fs::create_dir_all(dir).unwrap();
                let file = std::fs::File::create(dir.join(format!("scene-{t}s.png"))).unwrap();
                let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
                enc.set_color(png::ColorType::Rgb);
                enc.set_depth(png::BitDepth::Eight);
                let rgb: Vec<u8> = px
                    .iter()
                    .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
                    .collect();
                let mut writer = enc.write_header().unwrap();
                writer.write_image_data(&rgb).unwrap();
            }
        }
    }
}
