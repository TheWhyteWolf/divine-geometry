//! Five passes:
//!   P1 stroke + point → scene   (HDR, additive, cleared every frame)
//!   P2 bloom mip chain over scene
//!   P3 bloom composited back into scene (additive)
//!   P4 post grade → final_tex   (sRGB bytes)
//!   P5 blit → surface
//!
//! No feedback ping-pong and no kaleido, unlike the Colliderscope pipeline this
//! borrows from. A construction drawing needs a line drawn eight seconds ago to
//! look *identical* to one drawn now; under feedback it would be a decayed
//! smear. Persistence here comes from the step's `head` staying at 1.0 — exact
//! geometry, exact intensity, cheaper than a trail field.

use std::borrow::Cow;
use std::sync::Arc;

use crate::tess::{PointInstance, StepGpu, StrokeVertex, Tess};

// ------------------------------------------------------------ uniform structs

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SceneUniforms {
    pub res: [f32; 2],
    pub emissive: f32,
    pub time: f32,
    pub tint: [f32; 3],
    pub shimmer: f32,
    /// The camera. Lives in a uniform rather than baked into vertices, so
    /// zooming and panning never touch the vertex buffers.
    pub view_center: [f32; 2],
    pub view_scale: f32,
    pub _pad: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BloomUniforms {
    texel: [f32; 2],
    threshold: f32,
    knee: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CompositeUniforms {
    gain: f32,
    _pad: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PostUniforms {
    pub res: [f32; 2],
    pub bright: f32,
    pub vignette: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BlitUniforms {
    decode_srgb: u32,
    _pad: [u32; 3],
}

/// Everything the renderer needs per frame.
pub struct FrameInputs {
    pub scene: SceneUniforms,
    /// Identity-camera variant for the HUD, so text stays pixel-locked while
    /// the figure behind it zooms.
    pub hud: SceneUniforms,
    pub post: PostUniforms,
    pub bloom_gain: f32,
}

// ---------------------------------------------------------------- constants

const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const LDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
// Zero threshold: on a black field, thresholding would kill the halo on the
// faint scaffold, which is precisely the halo we want.
const BLOOM_THRESHOLD: f32 = 0.0;
const BLOOM_KNEE: f32 = 0.1;

const ADDITIVE: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
};

// ------------------------------------------------------------------- helpers

fn shader(device: &wgpu::Device, label: &str, body: &str) -> wgpu::ShaderModule {
    let src = format!("{}\n{}", include_str!("shaders/fullscreen.wgsl"), body);
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(Cow::Owned(src)),
    })
}

/// The common (uniform, texture, sampler) bind group layout for fullscreen passes.
fn uts_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("uts"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

fn uniform_buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn geo_buffer(
    device: &wgpu::Device,
    label: &str,
    size: u64,
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn uts_bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buf: &wgpu::Buffer,
    view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

fn fs_pipeline(
    device: &wgpu::Device,
    label: &str,
    module: &wgpu::ShaderModule,
    fs_entry: &str,
    layout: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
) -> wgpu::RenderPipeline {
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vs_fullscreen"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some(fs_entry),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}

fn make_tex(
    device: &wgpu::Device,
    label: &str,
    w: u32,
    h: u32,
    format: wgpu::TextureFormat,
    mips: u32,
    extra: wgpu::TextureUsages,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | extra,
        view_formats: &[],
    })
}

fn begin_pass<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    label: &str,
    view: &'e wgpu::TextureView,
    clear: bool,
) -> wgpu::RenderPass<'e> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: if clear {
                    wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                } else {
                    wgpu::LoadOp::Load
                },
                store: wgpu::StoreOp::Store,
            },
            depth_slice: None,
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

// --------------------------------------------------------------- the bundle

/// Everything tied to the surface resolution — rebuilt on resize.
struct Targets {
    scene_view: wgpu::TextureView,
    _scene: wgpu::Texture,
    _bloom: wgpu::Texture,
    bloom_mips: Vec<wgpu::TextureView>,
    _bloom_us: Vec<wgpu::Buffer>,
    prefilter_bg: wgpu::BindGroup,
    down_bg: Vec<wgpu::BindGroup>,
    up_bg: Vec<wgpu::BindGroup>,
    composite_bg: wgpu::BindGroup,
    final_view: wgpu::TextureView,
    final_tex: wgpu::Texture,
    post_bg: wgpu::BindGroup,
    blit_bg: wgpu::BindGroup,
}

#[allow(clippy::too_many_arguments)]
fn build_targets(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    uts: &wgpu::BindGroupLayout,
    samp: &wgpu::Sampler,
    composite_u: &wgpu::Buffer,
    post_u: &wgpu::Buffer,
    blit_u: &wgpu::Buffer,
    size: (u32, u32),
) -> Targets {
    let (w, h) = (size.0.max(8), size.1.max(8));
    let scene = make_tex(device, "scene", w, h, HDR, 1, wgpu::TextureUsages::empty());
    let scene_view = scene.create_view(&Default::default());

    // Bloom chain at half surface res, down to ~16 px on the short side.
    let (bw, bh) = ((w / 2).max(16), (h / 2).max(16));
    let mips = bw.min(bh).ilog2().saturating_sub(3).max(1);
    let bloom = make_tex(device, "bloom", bw, bh, HDR, mips, wgpu::TextureUsages::empty());
    let bloom_mips: Vec<_> = (0..mips)
        .map(|m| {
            bloom.create_view(&wgpu::TextureViewDescriptor {
                label: Some("bloom mip"),
                base_mip_level: m,
                mip_level_count: Some(1),
                ..Default::default()
            })
        })
        .collect();

    // bloom_us[0] samples the scene (prefilter); bloom_us[m + 1] samples mip m.
    let mut sizes = vec![(w, h)];
    for m in 0..mips {
        sizes.push(((bw >> m).max(1), (bh >> m).max(1)));
    }
    let bloom_us: Vec<_> = sizes
        .iter()
        .map(|(mw, mh)| {
            let buf = uniform_buffer(device, "bloom_u", 16);
            let u = BloomUniforms {
                texel: [1.0 / *mw as f32, 1.0 / *mh as f32],
                threshold: BLOOM_THRESHOLD,
                knee: BLOOM_KNEE,
            };
            queue.write_buffer(&buf, 0, bytemuck::bytes_of(&u));
            buf
        })
        .collect();

    let prefilter_bg = uts_bind(device, uts, &bloom_us[0], &scene_view, samp);
    let down_bg: Vec<_> = (0..mips.saturating_sub(1) as usize)
        .map(|m| uts_bind(device, uts, &bloom_us[m + 1], &bloom_mips[m], samp))
        .collect();
    let up_bg: Vec<_> = (1..mips as usize)
        .map(|m| uts_bind(device, uts, &bloom_us[m + 1], &bloom_mips[m], samp))
        .collect();
    let composite_bg = uts_bind(device, uts, composite_u, &bloom_mips[0], samp);

    let final_tex = make_tex(device, "final", w, h, LDR, 1, wgpu::TextureUsages::COPY_SRC);
    let final_view = final_tex.create_view(&Default::default());
    let post_bg = uts_bind(device, uts, post_u, &scene_view, samp);
    let blit_bg = uts_bind(device, uts, blit_u, &final_view, samp);

    Targets {
        scene_view,
        _scene: scene,
        _bloom: bloom,
        bloom_mips,
        _bloom_us: bloom_us,
        prefilter_bg,
        down_bg,
        up_bg,
        composite_bg,
        final_view,
        final_tex,
        post_bg,
        blit_bg,
    }
}

// -------------------------------------------------------------- the renderer

pub struct Renderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,

    stroke_p: wgpu::RenderPipeline,
    point_p: wgpu::RenderPipeline,
    prefilter_p: wgpu::RenderPipeline,
    down_p: wgpu::RenderPipeline,
    up_p: wgpu::RenderPipeline,
    composite_p: wgpu::RenderPipeline,
    post_p: wgpu::RenderPipeline,
    blit_p: wgpu::RenderPipeline,

    uts: wgpu::BindGroupLayout,
    scene_bgl: wgpu::BindGroupLayout,
    samp: wgpu::Sampler,

    scene_u: wgpu::Buffer,
    hud_u: wgpu::Buffer,
    composite_u: wgpu::Buffer,
    post_u: wgpu::Buffer,
    blit_u: wgpu::Buffer,
    step_sb: wgpu::Buffer,
    step_cap: u64,
    scene_bg: wgpu::BindGroup,
    hud_bg: wgpu::BindGroup,

    stroke_vb: wgpu::Buffer,
    stroke_ib: wgpu::Buffer,
    hud_vb: wgpu::Buffer,
    hud_ib: wgpu::Buffer,
    hud_vb_cap: u64,
    hud_ib_cap: u64,
    hud_index_count: u32,
    snow_vb: wgpu::Buffer,
    snow_vb_cap: u64,
    snow_count: u32,
    point_vb: wgpu::Buffer,
    stroke_vb_cap: u64,
    stroke_ib_cap: u64,
    point_vb_cap: u64,
    index_count: u32,
    point_count: u32,

    targets: Targets,
}

impl Renderer {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        size: (u32, u32),
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        let uts = uts_layout(&device);

        // Group 0 for the geometry passes: scene uniform + the per-step state
        // array. The step array is a storage buffer so the fragment shader can
        // index it by the flat-interpolated step_id.
        let scene_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let samp = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let stroke_m = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("stroke"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("shaders/stroke.wgsl"))),
        });
        let point_m = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("point"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("shaders/point.wgsl"))),
        });
        let bloom_m = shader(&device, "bloom", include_str!("shaders/bloom.wgsl"));
        let composite_m = shader(&device, "composite", include_str!("shaders/composite.wgsl"));
        let post_m = shader(&device, "post", include_str!("shaders/post.wgsl"));
        let blit_m = shader(&device, "blit", include_str!("shaders/blit.wgsl"));

        let prefilter_p =
            fs_pipeline(&device, "bloom-pre", &bloom_m, "fs_prefilter", &uts, HDR, None);
        let down_p = fs_pipeline(&device, "bloom-down", &bloom_m, "fs_down", &uts, HDR, None);
        let up_p = fs_pipeline(&device, "bloom-up", &bloom_m, "fs_up", &uts, HDR, Some(ADDITIVE));
        let composite_p = fs_pipeline(
            &device,
            "composite",
            &composite_m,
            "fs_composite",
            &uts,
            HDR,
            Some(ADDITIVE),
        );
        let post_p = fs_pipeline(&device, "post", &post_m, "fs_post", &uts, LDR, None);
        let blit_p = fs_pipeline(&device, "blit", &blit_m, "fs_blit", &uts, surface_format, None);

        let scene_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene"),
            bind_group_layouts: &[Some(&scene_bgl)],
            immediate_size: 0,
        });
        let stroke_p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("stroke"),
            layout: Some(&scene_pl),
            vertex: wgpu::VertexState {
                module: &stroke_m,
                entry_point: Some("vs_stroke"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<StrokeVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2, 1 => Float32x2, 2 => Float32, 3 => Float32,
                        4 => Float32, 5 => Float32, 6 => Float32, 7 => Uint32
                    ],
                }],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &stroke_m,
                entry_point: Some("fs_stroke"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR,
                    blend: Some(ADDITIVE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let point_p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("point"),
            layout: Some(&scene_pl),
            vertex: wgpu::VertexState {
                module: &point_m,
                entry_point: Some("vs_point"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<PointInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2, 1 => Float32, 2 => Float32, 3 => Float32
                    ],
                }],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &point_m,
                entry_point: Some("fs_point"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR,
                    blend: Some(ADDITIVE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let scene_u = uniform_buffer(&device, "scene_u", 48);
        let hud_u = uniform_buffer(&device, "hud_u", 48);
        let composite_u = uniform_buffer(&device, "composite_u", 16);
        let post_u = uniform_buffer(&device, "post_u", 16);
        let blit_u = uniform_buffer(&device, "blit_u", 16);

        let step_cap = 256 * std::mem::size_of::<StepGpu>() as u64;
        let step_sb = geo_buffer(&device, "step_sb", step_cap, wgpu::BufferUsages::STORAGE);
        let scene_bg = make_scene_bg(&device, &scene_bgl, &scene_u, &step_sb);
        let hud_bg = make_scene_bg(&device, &scene_bgl, &hud_u, &step_sb);

        let stroke_vb_cap = 1024 * 1024;
        let stroke_ib_cap = 1024 * 1024;
        let point_vb_cap = 128 * 1024;
        let stroke_vb = geo_buffer(&device, "stroke_vb", stroke_vb_cap, wgpu::BufferUsages::VERTEX);
        let stroke_ib = geo_buffer(&device, "stroke_ib", stroke_ib_cap, wgpu::BufferUsages::INDEX);
        let point_vb = geo_buffer(&device, "point_vb", point_vb_cap, wgpu::BufferUsages::VERTEX);
        let hud_vb_cap = 256 * 1024;
        let hud_ib_cap = 256 * 1024;
        let hud_vb = geo_buffer(&device, "hud_vb", hud_vb_cap, wgpu::BufferUsages::VERTEX);
        let hud_ib = geo_buffer(&device, "hud_ib", hud_ib_cap, wgpu::BufferUsages::INDEX);
        let snow_vb_cap = 32 * 1024;
        let snow_vb = geo_buffer(&device, "snow_vb", snow_vb_cap, wgpu::BufferUsages::VERTEX);

        let targets =
            build_targets(&device, &queue, &uts, &samp, &composite_u, &post_u, &blit_u, size);

        Self {
            device,
            queue,
            stroke_p,
            point_p,
            prefilter_p,
            down_p,
            up_p,
            composite_p,
            post_p,
            blit_p,
            uts,
            scene_bgl,
            samp,
            scene_u,
            hud_u,
            composite_u,
            post_u,
            blit_u,
            step_sb,
            step_cap,
            scene_bg,
            hud_bg,
            stroke_vb,
            stroke_ib,
            hud_vb,
            hud_ib,
            hud_vb_cap,
            hud_ib_cap,
            hud_index_count: 0,
            snow_vb,
            snow_vb_cap,
            snow_count: 0,
            point_vb,
            stroke_vb_cap,
            stroke_ib_cap,
            point_vb_cap,
            index_count: 0,
            point_count: 0,
            targets,
        }
    }

    pub fn resize(&mut self, size: (u32, u32)) {
        self.targets = build_targets(
            &self.device,
            &self.queue,
            &self.uts,
            &self.samp,
            &self.composite_u,
            &self.post_u,
            &self.blit_u,
            size,
        );
    }

    /// The graded sRGB-byte target — readback source for offscreen stills.
    pub fn final_tex(&self) -> &wgpu::Texture {
        &self.targets.final_tex
    }

    pub fn set_surface_srgb(&self, srgb: bool) {
        let u = BlitUniforms { decode_srgb: srgb as u32, _pad: [0; 3] };
        self.queue.write_buffer(&self.blit_u, 0, bytemuck::bytes_of(&u));
    }

    /// Upload static stroke geometry. Only on figure change or resize — the
    /// geometry itself never animates.
    pub fn upload_strokes(&mut self, t: &Tess) {
        let vb = bytemuck::cast_slice::<_, u8>(&t.verts);
        let ib = bytemuck::cast_slice::<_, u8>(&t.indices);

        if vb.len() as u64 > self.stroke_vb_cap {
            self.stroke_vb_cap = (vb.len() as u64).next_power_of_two();
            self.stroke_vb =
                geo_buffer(&self.device, "stroke_vb", self.stroke_vb_cap, wgpu::BufferUsages::VERTEX);
        }
        if ib.len() as u64 > self.stroke_ib_cap {
            self.stroke_ib_cap = (ib.len() as u64).next_power_of_two();
            self.stroke_ib =
                geo_buffer(&self.device, "stroke_ib", self.stroke_ib_cap, wgpu::BufferUsages::INDEX);
        }
        if !vb.is_empty() {
            self.queue.write_buffer(&self.stroke_vb, 0, vb);
        }
        if !ib.is_empty() {
            self.queue.write_buffer(&self.stroke_ib, 0, ib);
        }
        self.index_count = t.indices.len() as u32;
    }

    /// Node sprites — small enough to re-upload every frame, and their birth
    /// flash means they genuinely change.
    pub fn upload_points(&mut self, points: &[PointInstance]) {
        self.point_count = points.len() as u32;
        if points.is_empty() {
            return;
        }
        let pb = bytemuck::cast_slice::<_, u8>(points);
        if pb.len() as u64 > self.point_vb_cap {
            self.point_vb_cap = (pb.len() as u64).next_power_of_two();
            self.point_vb =
                geo_buffer(&self.device, "point_vb", self.point_vb_cap, wgpu::BufferUsages::VERTEX);
        }
        self.queue.write_buffer(&self.point_vb, 0, pb);
    }

    /// Upload this frame's per-step animation state — the only per-frame write.
    pub fn upload_steps(&mut self, steps: &[StepGpu]) {
        if steps.is_empty() {
            return;
        }
        let bytes = bytemuck::cast_slice::<_, u8>(steps);
        if bytes.len() as u64 > self.step_cap {
            self.step_cap = (bytes.len() as u64).next_power_of_two();
            self.step_sb =
                geo_buffer(&self.device, "step_sb", self.step_cap, wgpu::BufferUsages::STORAGE);
            // The buffer moved, so both bind groups referencing it are stale.
            self.scene_bg =
                make_scene_bg(&self.device, &self.scene_bgl, &self.scene_u, &self.step_sb);
            self.hud_bg =
                make_scene_bg(&self.device, &self.scene_bgl, &self.hud_u, &self.step_sb);
        }
        self.queue.write_buffer(&self.step_sb, 0, bytes);
    }

    /// Upload HUD geometry — rebuilt only when the readout text changes.
    pub fn upload_hud(&mut self, t: &Tess) {
        let vb = bytemuck::cast_slice::<_, u8>(&t.verts);
        let ib = bytemuck::cast_slice::<_, u8>(&t.indices);
        self.hud_index_count = t.indices.len() as u32;
        if vb.is_empty() || ib.is_empty() {
            return;
        }
        if vb.len() as u64 > self.hud_vb_cap {
            self.hud_vb_cap = (vb.len() as u64).next_power_of_two();
            self.hud_vb =
                geo_buffer(&self.device, "hud_vb", self.hud_vb_cap, wgpu::BufferUsages::VERTEX);
        }
        if ib.len() as u64 > self.hud_ib_cap {
            self.hud_ib_cap = (ib.len() as u64).next_power_of_two();
            self.hud_ib =
                geo_buffer(&self.device, "hud_ib", self.hud_ib_cap, wgpu::BufferUsages::INDEX);
        }
        self.queue.write_buffer(&self.hud_vb, 0, vb);
        self.queue.write_buffer(&self.hud_ib, 0, ib);
    }

    /// Glitter sprites — screen-space, so they ride the HUD's identity camera.
    pub fn upload_snow(&mut self, parts: &[PointInstance]) {
        self.snow_count = parts.len() as u32;
        if parts.is_empty() {
            return;
        }
        let pb = bytemuck::cast_slice::<_, u8>(parts);
        if pb.len() as u64 > self.snow_vb_cap {
            self.snow_vb_cap = (pb.len() as u64).next_power_of_two();
            self.snow_vb =
                geo_buffer(&self.device, "snow_vb", self.snow_vb_cap, wgpu::BufferUsages::VERTEX);
        }
        self.queue.write_buffer(&self.snow_vb, 0, pb);
    }

    /// Encode P1..P4.
    pub fn encode(&mut self, encoder: &mut wgpu::CommandEncoder, inputs: &FrameInputs) {
        self.queue.write_buffer(&self.scene_u, 0, bytemuck::bytes_of(&inputs.scene));
        self.queue.write_buffer(&self.hud_u, 0, bytemuck::bytes_of(&inputs.hud));
        self.queue.write_buffer(&self.post_u, 0, bytemuck::bytes_of(&inputs.post));
        // The up-chain accumulates every mip level, so normalize by the level
        // count — otherwise the halo strength scales with resolution.
        let mips = self.targets.bloom_mips.len();
        let comp = CompositeUniforms { gain: inputs.bloom_gain / mips as f32, _pad: [0.0; 3] };
        self.queue.write_buffer(&self.composite_u, 0, bytemuck::bytes_of(&comp));

        // P1: geometry → scene.
        {
            let mut pass = begin_pass(encoder, "scene", &self.targets.scene_view, true);
            if self.index_count > 0 {
                pass.set_pipeline(&self.stroke_p);
                pass.set_bind_group(0, &self.scene_bg, &[]);
                pass.set_vertex_buffer(0, self.stroke_vb.slice(..));
                pass.set_index_buffer(self.stroke_ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.index_count, 0, 0..1);
            }
            if self.point_count > 0 {
                pass.set_pipeline(&self.point_p);
                pass.set_bind_group(0, &self.scene_bg, &[]);
                pass.set_vertex_buffer(0, self.point_vb.slice(..));
                pass.draw(0..4, 0..self.point_count);
            }
            // Glitter falls in screen space, in front of the zooming figure:
            // the point pipeline again, but through the identity camera.
            if self.snow_count > 0 {
                pass.set_pipeline(&self.point_p);
                pass.set_bind_group(0, &self.hud_bg, &[]);
                pass.set_vertex_buffer(0, self.snow_vb.slice(..));
                pass.draw(0..4, 0..self.snow_count);
            }
            // The HUD shares the pipeline and the step buffer, and differs only
            // in its scene uniform: identity camera, no shimmer.
            if self.hud_index_count > 0 {
                pass.set_pipeline(&self.stroke_p);
                pass.set_bind_group(0, &self.hud_bg, &[]);
                pass.set_vertex_buffer(0, self.hud_vb.slice(..));
                pass.set_index_buffer(self.hud_ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..self.hud_index_count, 0, 0..1);
            }
        }

        // P2: bloom chain.
        {
            let mut pass = begin_pass(encoder, "bloom-pre", &self.targets.bloom_mips[0], true);
            pass.set_pipeline(&self.prefilter_p);
            pass.set_bind_group(0, &self.targets.prefilter_bg, &[]);
            pass.draw(0..3, 0..1);
        }
        for m in 0..mips.saturating_sub(1) {
            let mut pass = begin_pass(encoder, "bloom-down", &self.targets.bloom_mips[m + 1], true);
            pass.set_pipeline(&self.down_p);
            pass.set_bind_group(0, &self.targets.down_bg[m], &[]);
            pass.draw(0..3, 0..1);
        }
        for m in (0..mips.saturating_sub(1)).rev() {
            let mut pass = begin_pass(encoder, "bloom-up", &self.targets.bloom_mips[m], false);
            pass.set_pipeline(&self.up_p);
            pass.set_bind_group(0, &self.targets.up_bg[m], &[]);
            pass.draw(0..3, 0..1);
        }

        // P3: composite the halo back into the scene.
        {
            let mut pass = begin_pass(encoder, "composite", &self.targets.scene_view, false);
            pass.set_pipeline(&self.composite_p);
            pass.set_bind_group(0, &self.targets.composite_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // P4: post grade → final_tex.
        {
            let mut pass = begin_pass(encoder, "post", &self.targets.final_view, true);
            pass.set_pipeline(&self.post_p);
            pass.set_bind_group(0, &self.targets.post_bg, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// P5: draw final_tex into the given surface pass.
    pub fn blit(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.blit_p);
        pass.set_bind_group(0, &self.targets.blit_bg, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn make_scene_bg(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    scene_u: &wgpu::Buffer,
    step_sb: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("scene"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: scene_u.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: step_sb.as_entire_binding() },
        ],
    })
}
