//! wgpu renderer: textured meshes (level, objects, first person weapon),
//! alpha-blended effect sprites, and the 2D HUD.

use crate::scene::{mip_chain, Scene, Vertex};
use blam_cache::bitmap::Image;
use glam::{Mat4, Vec3};
use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::window::Window;

/// Per-draw uniforms; each draw gets its own 256-byte slot (dynamic offset).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DrawUniforms {
    mvp: [[f32; 4]; 4],
    model: [[f32; 4]; 4],
    camera: [f32; 4],
    /// x: fog amount, y/z: screen size (HUD).
    params: [f32; 4],
}

const SLOT: u64 = 256;
const MAX_DRAWS: u64 = 1024;

/// A coloured, textured point of an effect (decal, flash, puff) in world space.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SpriteVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

/// A HUD vertex in pixels from the top-left of the window.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HudVertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    /// x: shading mode (see HUD shader), y: meter value.
    pub mode: [f32; 2],
}

pub mod hud_mode {
    /// Texture times colour.
    pub const PLAIN: f32 = 0.0;
    /// Halo 2 HUD art: blue channel is the bright line, green the fill.
    pub const CHANNELS: f32 = 1.0;
    /// Meter whose blue channel orders the cells, green is the shape.
    pub const METER_BLUE: f32 = 2.0;
    /// Meter whose grey level orders the fill.
    pub const METER_GREY: f32 = 3.0;
}

pub struct DrawCall {
    pub mesh: usize,
    pub model: Mat4,
}

pub struct HudBatch {
    pub texture: usize,
    pub vertices: Vec<HudVertex>,
}

pub struct Frame<'a> {
    pub view_proj: Mat4,
    pub camera: Vec3,
    pub world: &'a [DrawCall],
    pub sprites: &'a [SpriteVertex],
    /// Drawn after the world with a fresh depth buffer so it never clips walls.
    pub view_model_proj: Mat4,
    pub view_models: &'a [DrawCall],
    pub view_sprites: &'a [SpriteVertex],
    pub hud: &'a [HudBatch],
}

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SKY: wgpu::Color = wgpu::Color {
    r: 0.62,
    g: 0.70,
    b: 0.80,
    a: 1.0,
};

const MESH_SHADER: &str = r#"
struct U {
    mvp: mat4x4<f32>,
    model: mat4x4<f32>,
    camera: vec4<f32>,
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: U;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;
@group(1) @binding(2) var lightmap: texture_2d<f32>;
@group(1) @binding(3) var lsamp: sampler;

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
    let albedo = textureSample(tex, samp, i.uv).rgb;
    let baked = textureSample(lightmap, lsamp, i.lmuv).rgb;
    var light: vec3<f32>;
    if (u.params.w > 0.5) {
        // Level geometry: its lightmap page, or per-vertex colour.
        // Lightmap pages are decoded as sRGB; vertex colours arrive in gamma space.
        let colour = pow(i.light.rgb, vec3<f32>(2.2));
        light = colour * mix(vec3<f32>(1.0), baked, i.light.a) * LIGHTMAP_SCALE;
    } else {
        // Objects: sky-from-above / ground-bounce ambient plus a soft sun.
        let n = normalize(i.normal);
        let sun = normalize(vec3<f32>(0.4, -0.3, 0.85));
        let ambient = mix(vec3<f32>(0.32, 0.30, 0.28), vec3<f32>(0.55, 0.60, 0.68), n.z * 0.5 + 0.5);
        light = ambient + vec3<f32>(0.75, 0.72, 0.65) * max(dot(n, sun), 0.0);
    }
    let fog = clamp(distance(i.world, u.camera.xyz) / 400.0, 0.0, 1.0) * u.params.x;
    let sky = vec3<f32>(0.62, 0.70, 0.80);
    return vec4<f32>(mix(albedo * light, sky, fog * fog), 1.0);
}
"#;

const SPRITE_SHADER: &str = r#"
struct U {
    mvp: mat4x4<f32>,
    model: mat4x4<f32>,
    camera: vec4<f32>,
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: U;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>) -> Out {
    var o: Out;
    o.clip = u.mvp * vec4<f32>(position, 1.0);
    o.uv = uv;
    o.color = color;
    return o;
}

@fragment
fn fs(i: Out) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, i.uv) * i.color;
}
"#;

const HUD_SHADER: &str = r#"
struct U {
    mvp: mat4x4<f32>,
    model: mat4x4<f32>,
    camera: vec4<f32>,
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: U;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) mode: vec2<f32>,
};

@vertex
fn vs(@location(0) position: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>, @location(3) mode: vec2<f32>) -> Out {
    var o: Out;
    let ndc = vec2<f32>(position.x / u.params.y * 2.0 - 1.0, 1.0 - position.y / u.params.z * 2.0);
    o.clip = vec4<f32>(ndc, 0.0, 1.0);
    o.uv = uv;
    o.color = color;
    o.mode = mode;
    return o;
}

@fragment
fn fs(i: Out) -> @location(0) vec4<f32> {
    let t = textureSample(tex, samp, i.uv);
    let mode = i32(i.mode.x + 0.5);
    if (mode == 1) {
        // Bright lines in blue, translucent fill in green.
        let line = smoothstep(0.3, 0.7, t.b);
        let k = max(t.b, t.g * 0.45);
        return vec4<f32>(i.color.rgb * k, t.a * i.color.a * mix(0.55, 1.0, line));
    }
    if (mode == 2) {
        // Cells whose order value is above the meter are spent: draw them faint.
        let on = step(t.b, i.mode.y + 0.002);
        return vec4<f32>(i.color.rgb * t.g, t.a * i.color.a * mix(0.12, 1.0, on));
    }
    if (mode == 3) {
        let on = step(t.r, i.mode.y + 0.002);
        return vec4<f32>(i.color.rgb, t.a * i.color.a * on);
    }
    return t * i.color;
}
"#;

pub struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    /// Material bind group and index range of each batch.
    batches: Vec<(usize, std::ops::Range<u32>)>,
    baked: bool,
}

pub struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    mesh_pipeline: wgpu::RenderPipeline,
    sprite_pipeline: wgpu::RenderPipeline,
    hud_pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    globals: wgpu::BindGroup,
    /// One bind group per (texture, lightmap) pair the meshes use.
    materials: Vec<wgpu::BindGroup>,
    hud_textures: Vec<wgpu::BindGroup>,
    effects_texture: wgpu::BindGroup,
    meshes: Vec<GpuMesh>,
    depth: wgpu::TextureView,
    staging: Vec<u8>,
}

fn depth_view(device: &wgpu::Device, w: u32, h: u32) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    img: &Image,
    srgb_mips: bool,
) -> wgpu::BindGroup {
    let view = upload_view(device, queue, img, srgb_mips);
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn upload_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    img: &Image,
    srgb_mips: bool,
) -> wgpu::TextureView {
    let mips = if srgb_mips {
        mip_chain(img)
    } else {
        vec![(img.width, img.height, img.rgba.clone())]
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: img.width,
            height: img.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: mips.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: if srgb_mips {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        },
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, (w, h, data)) in mips.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(*h),
            },
            wgpu::Extent3d {
                width: *w,
                height: *h,
                depth_or_array_layers: 1,
            },
        );
    }
    texture.create_view(&wgpu::TextureViewDescriptor::default())
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

/// A soft white disc: alpha falls off from the centre.
fn effects_image() -> Image {
    let n = 64u32;
    let mut rgba = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let dx = (x as f32 + 0.5) / n as f32 * 2.0 - 1.0;
            let dy = (y as f32 + 0.5) / n as f32 * 2.0 - 1.0;
            let r = (dx * dx + dy * dy).sqrt();
            let a = (1.0 - r).clamp(0.0, 1.0).powf(0.8);
            rgba.extend_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
        }
    }
    Image {
        width: n,
        height: n,
        rgba,
    }
}

fn blended(format: wgpu::TextureFormat) -> [Option<wgpu::ColorTargetState>; 1] {
    [Some(wgpu::ColorTargetState {
        format,
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
        write_mask: wgpu::ColorWrites::ALL,
    })]
}

impl Gpu {
    pub async fn new(
        window: Arc<Window>,
        scene: &Scene,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let size = window.inner_size();
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance.create_surface(window)?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await?;
        println!("gpu: {}", adapter.get_info().name);
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults()
                    .using_resolution(adapter.limits()),
                ..Default::default()
            })
            .await?;

        let config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("surface not supported by this GPU")?;
        surface.configure(&device, &config);

        let mut material_index = std::collections::HashMap::new();
        let mut material_keys: Vec<(usize, usize)> = Vec::new();
        let meshes: Vec<GpuMesh> = scene
            .meshes
            .iter()
            .map(|m| GpuMesh {
                vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("vertices"),
                    contents: bytemuck::cast_slice(&m.vertices),
                    // Skinned meshes are re-posed every frame.
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                }),
                indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("indices"),
                    contents: bytemuck::cast_slice(&m.indices),
                    usage: wgpu::BufferUsages::INDEX,
                }),
                batches: m
                    .batches
                    .iter()
                    .map(|b| {
                        let key = (b.texture, b.lightmap);
                        let next = material_keys.len();
                        let index = *material_index.entry(key).or_insert(next);
                        if index == next {
                            material_keys.push(key);
                        }
                        (index, b.first_index..b.first_index + b.index_count)
                    })
                    .collect(),
                baked: m.baked_lighting,
            })
            .collect();
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("draw uniforms"),
            size: SLOT * MAX_DRAWS,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_size = wgpu::BufferSize::new(std::mem::size_of::<DrawUniforms>() as u64);

        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: uniform_size,
                },
                count: None,
            }],
        });
        let globals = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniforms,
                    offset: 0,
                    size: uniform_size,
                }),
            }],
        });

        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mesh material"),
            entries: &[
                texture_entry(0),
                sampler_entry(1),
                texture_entry(2),
                sampler_entry(3),
            ],
        });
        let views: Vec<wgpu::TextureView> = scene
            .textures
            .iter()
            .map(|img| upload_view(&device, &queue, img, true))
            .collect();
        let materials = material_keys
            .iter()
            .map(|&(t, l)| {
                let view = |i: usize| views.get(i).unwrap_or(&views[0]);
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &material_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(view(t)),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&repeat),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(view(l)),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::Sampler(&clamp),
                        },
                    ],
                })
            })
            .collect();
        let hud_textures = scene
            .hud_textures
            .iter()
            .map(|img| upload_texture(&device, &queue, &texture_layout, &clamp, img, false))
            .collect();
        let effects_texture = upload_texture(
            &device,
            &queue,
            &texture_layout,
            &clamp,
            &effects_image(),
            false,
        );

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&globals_layout), Some(&texture_layout)],
            immediate_size: 0,
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
        let sprite_shader = module("sprite", SPRITE_SHADER);
        let hud_shader = module("hud", HUD_SHADER);

        let mesh_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("mesh"),
            layout: Some(&mesh_layout),
            vertex: wgpu::VertexState {
                module: &mesh_shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
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
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &mesh_shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(config.format.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sprite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sprites"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &sprite_shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<SpriteVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4],
                })],
            },
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &sprite_shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &blended(config.format),
            }),
            multiview_mask: None,
            cache: None,
        });
        let hud_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hud"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &hud_shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<HudVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x2],
                })],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &hud_shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &blended(config.format),
            }),
            multiview_mask: None,
            cache: None,
        });

        let depth = depth_view(&device, config.width, config.height);
        Ok(Gpu {
            surface,
            device,
            queue,
            config,
            mesh_pipeline,
            sprite_pipeline,
            hud_pipeline,
            uniforms,
            globals,
            materials,
            hud_textures,
            effects_texture,
            meshes,
            depth,
            staging: Vec::new(),
        })
    }

    pub fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height.max(1) as f32
    }

    pub fn size(&self) -> (f32, f32) {
        (self.config.width as f32, self.config.height as f32)
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
        self.depth = depth_view(&self.device, w, h);
    }

    /// Queue a uniform slot; returns its dynamic offset.
    fn slot(
        &mut self,
        proj: Mat4,
        model: Mat4,
        camera: Vec3,
        fog: f32,
        baked: bool,
    ) -> Option<u32> {
        let offset = self.staging.len() as u64;
        if offset / SLOT >= MAX_DRAWS {
            return None;
        }
        let (w, h) = self.size();
        let u = DrawUniforms {
            mvp: (proj * model).to_cols_array_2d(),
            model: model.to_cols_array_2d(),
            camera: camera.extend(1.0).into(),
            params: [fog, w, h, if baked { 1.0 } else { 0.0 }],
        };
        self.staging.extend_from_slice(bytemuck::bytes_of(&u));
        self.staging.resize((offset + SLOT) as usize, 0);
        Some(offset as u32)
    }

    /// Replace a mesh's vertices (same count) with a newly posed set.
    pub fn update_mesh(&self, mesh: usize, vertices: &[Vertex]) {
        if let Some(m) = self.meshes.get(mesh) {
            let bytes: &[u8] = bytemuck::cast_slice(vertices);
            if bytes.len() as u64 <= m.vertices.size() && !bytes.is_empty() {
                self.queue.write_buffer(&m.vertices, 0, bytes);
            }
        }
    }

    fn vertex_buffer<T: bytemuck::Pod>(&self, v: &[T]) -> Option<wgpu::Buffer> {
        (!v.is_empty()).then(|| {
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(v),
                    usage: wgpu::BufferUsages::VERTEX,
                })
        })
    }

    pub fn render(&mut self, f: &Frame) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            _ => return,
        };
        self.staging.clear();
        let cam = f.camera;
        let mut world = Vec::new();
        for d in f.world {
            let baked = self.meshes.get(d.mesh).is_some_and(|m| m.baked);
            if let Some(o) = self.slot(f.view_proj, d.model, cam, 1.0, baked) {
                world.push((d.mesh, o));
            }
        }
        let sprites_slot = self.slot(f.view_proj, Mat4::IDENTITY, cam, 0.0, false);
        let mut views = Vec::new();
        for d in f.view_models {
            if let Some(o) = self.slot(f.view_model_proj, d.model, cam, 0.0, false) {
                views.push((d.mesh, o));
            }
        }
        let view_sprites_slot = self.slot(f.view_model_proj, Mat4::IDENTITY, cam, 0.0, false);
        let hud_slot = self.slot(Mat4::IDENTITY, Mat4::IDENTITY, cam, 0.0, false);
        self.queue.write_buffer(&self.uniforms, 0, &self.staging);

        let sprites = self.vertex_buffer(f.sprites);
        let view_sprites = self.vertex_buffer(f.view_sprites);
        let hud: Vec<(usize, wgpu::Buffer, u32)> = f
            .hud
            .iter()
            .filter(|b| b.texture < self.hud_textures.len())
            .filter_map(|b| {
                Some((
                    b.texture,
                    self.vertex_buffer(&b.vertices)?,
                    b.vertices.len() as u32,
                ))
            })
            .collect();

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let color = |clear: Option<wgpu::Color>| wgpu::RenderPassColorAttachment {
            view: &view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: clear.map_or(wgpu::LoadOp::Load, wgpu::LoadOp::Clear),
                store: wgpu::StoreOp::Store,
            },
        };
        let depth = || wgpu::RenderPassDepthStencilAttachment {
            view: &self.depth,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(0.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        };
        // The world, then effects in it.
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("world"),
                color_attachments: &[Some(color(Some(SKY)))],
                depth_stencil_attachment: Some(depth()),
                ..Default::default()
            });
            pass.set_pipeline(&self.mesh_pipeline);
            for (mesh, offset) in &world {
                self.draw_mesh(&mut pass, *mesh, *offset);
            }
            if let (Some(buf), Some(offset)) = (&sprites, sprites_slot) {
                self.draw_sprites(&mut pass, buf, f.sprites.len(), offset);
            }
        }
        // First person weapon on top of the world.
        if !views.is_empty() || view_sprites.is_some() {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("view model"),
                color_attachments: &[Some(color(None))],
                depth_stencil_attachment: Some(depth()),
                ..Default::default()
            });
            pass.set_pipeline(&self.mesh_pipeline);
            for (mesh, offset) in &views {
                self.draw_mesh(&mut pass, *mesh, *offset);
            }
            if let (Some(buf), Some(offset)) = (&view_sprites, view_sprites_slot) {
                self.draw_sprites(&mut pass, buf, f.view_sprites.len(), offset);
            }
        }
        // HUD.
        if let (false, Some(offset)) = (hud.is_empty(), hud_slot) {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hud"),
                color_attachments: &[Some(color(None))],
                ..Default::default()
            });
            pass.set_pipeline(&self.hud_pipeline);
            pass.set_bind_group(0, &self.globals, &[offset]);
            for (texture, buf, count) in &hud {
                pass.set_bind_group(1, &self.hud_textures[*texture], &[]);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..*count, 0..1);
            }
        }
        self.queue.submit([enc.finish()]);
        self.queue.present(frame);
    }

    fn draw_mesh(&self, pass: &mut wgpu::RenderPass, mesh: usize, offset: u32) {
        let Some(m) = self.meshes.get(mesh) else {
            return;
        };
        pass.set_bind_group(0, &self.globals, &[offset]);
        pass.set_vertex_buffer(0, m.vertices.slice(..));
        pass.set_index_buffer(m.indices.slice(..), wgpu::IndexFormat::Uint32);
        for (material, range) in &m.batches {
            pass.set_bind_group(1, &self.materials[*material], &[]);
            pass.draw_indexed(range.clone(), 0, 0..1);
        }
    }

    fn draw_sprites(&self, pass: &mut wgpu::RenderPass, buf: &wgpu::Buffer, n: usize, offset: u32) {
        pass.set_pipeline(&self.sprite_pipeline);
        pass.set_bind_group(0, &self.globals, &[offset]);
        pass.set_bind_group(1, &self.effects_texture, &[]);
        pass.set_vertex_buffer(0, buf.slice(..));
        pass.draw(0..n as u32, 0..1);
    }
}
