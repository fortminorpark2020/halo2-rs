//! wgpu renderer for the level mesh: flat-shaded triangles with depth and fog.

use blam_cache::geometry::Mesh;
use glam::{Mat4, Vec3};
use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::window::Window;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    camera: [f32; 4],
    /// x: min height, y: 1 / height range
    height: [f32; 4],
}

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

const SHADER: &str = r#"
struct Uniforms {
    view_proj: mat4x4<f32>,
    camera: vec4<f32>,
    height: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

@vertex
fn vs(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>) -> Out {
    var o: Out;
    o.clip = u.view_proj * vec4<f32>(position, 1.0);
    o.world = position;
    o.normal = normal;
    return o;
}

@fragment
fn fs(i: Out) -> @location(0) vec4<f32> {
    let light = normalize(vec3<f32>(0.4, -0.3, 0.85));
    let n = normalize(i.normal);
    let diffuse = 0.3 + 0.7 * abs(dot(n, light));
    let h = clamp((i.world.z - u.height.x) * u.height.y, 0.0, 1.0);
    // Floors warm, walls cool, brighter with height.
    let floor = smoothstep(0.5, 0.9, n.z);
    let wall = mix(vec3<f32>(0.42, 0.50, 0.62), vec3<f32>(0.62, 0.68, 0.78), h);
    let ground = mix(vec3<f32>(0.62, 0.56, 0.42), vec3<f32>(0.88, 0.84, 0.70), h);
    let base = mix(wall, ground, floor);
    // Faint grid every world unit helps judge scale and motion.
    let g = abs(fract(i.world - 0.5) - 0.5);
    let line = step(min(min(g.x, g.y), g.z), 0.015) * 0.08;
    let fog = clamp(distance(i.world, u.camera.xyz) / 300.0, 0.0, 1.0);
    let sky = vec3<f32>(0.62, 0.70, 0.80);
    return vec4<f32>(mix(base * diffuse - line, sky, fog * fog), 1.0);
}
"#;

pub struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
    vertex_count: u32,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    depth: wgpu::TextureView,
    height: [f32; 4],
}

fn flat_vertices(mesh: &Mesh) -> Vec<Vertex> {
    let mut out = Vec::with_capacity(mesh.indices.len());
    for t in mesh.indices.chunks_exact(3) {
        let p = [t[0], t[1], t[2]].map(|i| Vec3::from(mesh.positions[i as usize]));
        let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or(Vec3::Z);
        for v in p {
            out.push(Vertex {
                position: v.into(),
                normal: n.into(),
            });
        }
    }
    out
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

impl Gpu {
    pub async fn new(window: Arc<Window>, mesh: &Mesh) -> Result<Self, Box<dyn std::error::Error>> {
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

        let verts = flat_vertices(mesh);
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("level vertices"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let (lo, hi) = mesh.bounds().unwrap_or(([0.0; 3], [1.0; 3]));
        // Ignore the kill floor when picking the colour ramp.
        let mut zs: Vec<f32> = mesh.positions.iter().map(|p| p[2]).collect();
        zs.sort_by(f32::total_cmp);
        let zlo = zs[zs.len() / 20].max(lo[2]);
        let height = [zlo, 1.0 / (hi[2] - zlo).max(1e-3), 0.0, 0.0];

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("level"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("level"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3],
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
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(config.format.into())],
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
            pipeline,
            vertices,
            vertex_count: verts.len() as u32,
            uniforms,
            bind_group,
            depth,
            height,
        })
    }

    pub fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height.max(1) as f32
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

    pub fn render(&mut self, view_proj: Mat4, camera: Vec3) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f)
            | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            _ => return,
        };
        let u = Uniforms {
            view_proj: view_proj.to_cols_array_2d(),
            camera: camera.extend(1.0).into(),
            height: self.height,
        };
        self.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.62,
                            g: 0.70,
                            b: 0.80,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            pass.draw(0..self.vertex_count, 0..1);
        }
        self.queue.submit([enc.finish()]);
        self.queue.present(frame);
    }
}
