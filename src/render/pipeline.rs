//! The passes, in order:
//!
//!   P1  stroke + point → scene   (HDR, additive, cleared every frame)
//!   P2  bloom mip chain over scene
//!   P3  bloom composited back into scene (additive)
//!   P3b history[next] = feedback(history[prev]) + scene         if trails on
//!   P3c background field → the frame (additive; via compose if trails) if bg on
//!   P4  foreground remap → fx_tex                               if fg on
//!   P5  post grade → final_tex   (sRGB bytes)
//!   P6  blit → surface
//!
//! P3b, P3c and P4 are the psychedelic layers, and all three default to off.
//! Off is structural, not a uniform set to zero: the pass is never encoded, its
//! textures are never written, and post binds the scene directly. So the clean
//! build renders exactly the five passes it always did, down to the byte.
//!
//! That matters because the original argument still stands. A construction
//! drawing needs a line drawn eight seconds ago to look *identical* to one drawn
//! now; under feedback it would be a decayed smear. Persistence in the default
//! build comes from the step's `head` staying at 1.0 — exact geometry, exact
//! intensity, cheaper than a trail field. The trail field is what you get when
//! you ask for it, and even then the degenerate case is honest: at `keep = 0`
//! the history pass writes `0·feedback + scene`, so the history *is* the scene.
//!
//! Two structural facts keep the feedback loop from running away. Bloom is
//! computed over `scene` only and never over the history field, so the loop
//! carries no halo gain; and the feedback shader clamps its output.

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

/// Background field. 64 bytes; the layout is hand-matched to `field.wgsl` —
/// `tint` is a `vec3<f32>` and so must land on a 16-byte boundary.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FieldUniforms {
    pub res: [f32; 2],
    pub time: f32,
    pub gain: f32,
    pub tint: [f32; 3],
    pub scale: f32,
    pub hue: f32,
    pub sat: f32,
    pub seg: f32,
    pub warp: f32,
    pub frac_c: [f32; 2],
    pub frac_iter: f32,
    pub mode: u32,
}

/// Foreground remap. 48 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FxUniforms {
    pub res: [f32; 2],
    pub time: f32,
    pub amount: f32,
    pub seg: f32,
    pub rot: f32,
    pub zoom: f32,
    pub swirl: f32,
    pub warp: f32,
    pub chroma: f32,
    pub scroll: f32,
    pub mode: u32,
}

/// Trail feedback. Every field arrives already framerate-corrected — see
/// `Fx::feedback_uniforms`.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FeedbackUniforms {
    pub keep: f32,
    pub flow_alpha: f32,
    pub rot: f32,
    pub inv_scale: f32,
    pub clamp_max: f32,
    pub _pad: [f32; 3],
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
    /// The background field. Its pass is encoded only while `bg_on`.
    pub field: FieldUniforms,
    pub bg_on: bool,
    /// The foreground remap. Its pass is encoded only while `fg_on`.
    pub fx: FxUniforms,
    pub fg_on: bool,
    /// Trail feedback. Its pass is encoded only while `trails_on`.
    pub feedback: FeedbackUniforms,
    pub trails_on: bool,
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

/// Bind group layout for a pass that binds nothing but a uniform block. The
/// background field generates its image from position and time, so it has no
/// source texture to sample and cannot use `uts_layout`.
fn u_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("u"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
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

fn u_bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buf: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }],
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

/// The uniform buffers every fullscreen pass binds. Grouped because
/// `build_targets` needs all of them at once to rebuild its bind groups, and a
/// nine-argument function is worse than a struct.
struct FsBufs<'a> {
    composite: &'a wgpu::Buffer,
    /// A `CompositeUniforms` pinned at gain 1 — the additive straight copy of
    /// the scene into the trail field.
    one: &'a wgpu::Buffer,
    post: &'a wgpu::Buffer,
    blit: &'a wgpu::Buffer,
    fx: &'a wgpu::Buffer,
    feedback: &'a wgpu::Buffer,
}

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

    /// Trail history, ping-ponged. Untouched unless trails are on.
    history: [wgpu::TextureView; 2],
    _history: [wgpu::Texture; 2],
    /// Reads history[i] to write the decayed field into the other one.
    feedback_bg: [wgpu::BindGroup; 2],
    /// The scene, bound at gain 1 for the additive copy into the history field.
    scene_copy_bg: wgpu::BindGroup,

    /// Where the trail field and the background field are summed, when both are
    /// on. It exists because the sum must not be visible to the feedback loop:
    /// the field is near-static, so anything that let it back into the history
    /// would accumulate it toward `gain / (1 - keep)` and wash the frame out.
    compose_view: wgpu::TextureView,
    _compose: wgpu::Texture,
    /// Copies history[i] into the compose target at gain 1.
    hist_copy_bg: [wgpu::BindGroup; 2],

    /// Foreground output, plus the three textures it might be asked to read.
    fx_view: wgpu::TextureView,
    _fx: wgpu::Texture,
    fx_bg_scene: wgpu::BindGroup,
    fx_bg_hist: [wgpu::BindGroup; 2],
    fx_bg_compose: wgpu::BindGroup,

    final_view: wgpu::TextureView,
    final_tex: wgpu::Texture,
    /// Post reads wherever the enabled layers left the frame: the scene, the
    /// trail field, or the foreground output. Choosing a bind group is what
    /// makes "off" cost nothing — there is no branch inside the shader.
    post_bg: wgpu::BindGroup,
    post_bg_hist: [wgpu::BindGroup; 2],
    post_bg_compose: wgpu::BindGroup,
    post_bg_fx: wgpu::BindGroup,
    blit_bg: wgpu::BindGroup,
}

fn build_targets(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    uts: &wgpu::BindGroupLayout,
    samp: &wgpu::Sampler,
    mirror: &wgpu::Sampler,
    bufs: &FsBufs<'_>,
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
    let composite_bg = uts_bind(device, uts, bufs.composite, &bloom_mips[0], samp);

    // The FX targets are full-resolution HDR, same as the scene: two halves of
    // the trail ping-pong, the compose buffer, and the foreground output. They
    // are allocated whether or not the layers are on — a toggle should not stall
    // on an allocation — which costs 32 bytes per pixel standing, about 64 MB at
    // 1080p, alongside the bloom chain that is already there.
    let history = [
        make_tex(device, "trail-a", w, h, HDR, 1, wgpu::TextureUsages::empty()),
        make_tex(device, "trail-b", w, h, HDR, 1, wgpu::TextureUsages::empty()),
    ];
    let history_views = [
        history[0].create_view(&Default::default()),
        history[1].create_view(&Default::default()),
    ];
    let feedback_bg = [
        uts_bind(device, uts, bufs.feedback, &history_views[0], samp),
        uts_bind(device, uts, bufs.feedback, &history_views[1], samp),
    ];
    let scene_copy_bg = uts_bind(device, uts, bufs.one, &scene_view, samp);

    let compose = make_tex(device, "compose", w, h, HDR, 1, wgpu::TextureUsages::empty());
    let compose_view = compose.create_view(&Default::default());
    let hist_copy_bg = [
        uts_bind(device, uts, bufs.one, &history_views[0], samp),
        uts_bind(device, uts, bufs.one, &history_views[1], samp),
    ];

    // The foreground samples through the mirror-repeat sampler: wrapping by
    // reflection *is* a kaleidoscope's edge behaviour, done in hardware.
    let fx_tex = make_tex(device, "fx", w, h, HDR, 1, wgpu::TextureUsages::empty());
    let fx_view = fx_tex.create_view(&Default::default());
    let fx_bg_scene = uts_bind(device, uts, bufs.fx, &scene_view, mirror);
    let fx_bg_hist = [
        uts_bind(device, uts, bufs.fx, &history_views[0], mirror),
        uts_bind(device, uts, bufs.fx, &history_views[1], mirror),
    ];
    let fx_bg_compose = uts_bind(device, uts, bufs.fx, &compose_view, mirror);

    let final_tex = make_tex(device, "final", w, h, LDR, 1, wgpu::TextureUsages::COPY_SRC);
    let final_view = final_tex.create_view(&Default::default());
    let post_bg = uts_bind(device, uts, bufs.post, &scene_view, samp);
    let post_bg_hist = [
        uts_bind(device, uts, bufs.post, &history_views[0], samp),
        uts_bind(device, uts, bufs.post, &history_views[1], samp),
    ];
    let post_bg_compose = uts_bind(device, uts, bufs.post, &compose_view, samp);
    let post_bg_fx = uts_bind(device, uts, bufs.post, &fx_view, samp);
    let blit_bg = uts_bind(device, uts, bufs.blit, &final_view, samp);

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
        history: history_views,
        _history: history,
        feedback_bg,
        scene_copy_bg,
        compose_view,
        _compose: compose,
        hist_copy_bg,
        fx_view,
        _fx: fx_tex,
        fx_bg_scene,
        fx_bg_hist,
        fx_bg_compose,
        final_view,
        final_tex,
        post_bg,
        post_bg_hist,
        post_bg_compose,
        post_bg_fx,
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
    field_p: wgpu::RenderPipeline,
    feedback_p: wgpu::RenderPipeline,
    fx_p: wgpu::RenderPipeline,
    post_p: wgpu::RenderPipeline,
    blit_p: wgpu::RenderPipeline,

    uts: wgpu::BindGroupLayout,
    scene_bgl: wgpu::BindGroupLayout,
    samp: wgpu::Sampler,
    mirror: wgpu::Sampler,

    scene_u: wgpu::Buffer,
    hud_u: wgpu::Buffer,
    composite_u: wgpu::Buffer,
    one_u: wgpu::Buffer,
    post_u: wgpu::Buffer,
    blit_u: wgpu::Buffer,
    field_u: wgpu::Buffer,
    feedback_u: wgpu::Buffer,
    fx_u: wgpu::Buffer,
    field_bg: wgpu::BindGroup,
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

    /// Which half of the trail ping-pong holds the current field.
    cur: usize,
    /// Trails on last frame. A layer switched back on would otherwise reveal a
    /// frozen ghost of whatever was on screen when it was switched off.
    trails_were_on: bool,

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
        let u_bgl = u_layout(&device);

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
        // The foreground fold reads far outside 0..1 by design. Reflecting at
        // the edge is the mirror a kaleidoscope is made of, so let the hardware
        // be it rather than open-coding the fold in the shader.
        let mirror = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("mirror"),
            address_mode_u: wgpu::AddressMode::MirrorRepeat,
            address_mode_v: wgpu::AddressMode::MirrorRepeat,
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
        let field_m = shader(&device, "field", include_str!("shaders/field.wgsl"));
        let feedback_m = shader(&device, "feedback", include_str!("shaders/feedback.wgsl"));
        let fx_m = shader(&device, "fx", include_str!("shaders/fx.wgsl"));
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
        // The FX pipelines are built whether or not the layers are ever switched
        // on. Naga validates at pipeline creation, so a broken shader fails at
        // start-up — including in `--shot`, which is the verification path —
        // rather than the first time somebody presses a key.
        let field_p =
            fs_pipeline(&device, "field", &field_m, "fs_field", &u_bgl, HDR, Some(ADDITIVE));
        let feedback_p =
            fs_pipeline(&device, "feedback", &feedback_m, "fs_feedback", &uts, HDR, None);
        let fx_p = fs_pipeline(&device, "fx", &fx_m, "fs_fx", &uts, HDR, None);
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
        let one_u = uniform_buffer(&device, "one_u", 16);
        queue.write_buffer(
            &one_u,
            0,
            bytemuck::bytes_of(&CompositeUniforms { gain: 1.0, _pad: [0.0; 3] }),
        );
        let post_u = uniform_buffer(&device, "post_u", 16);
        let blit_u = uniform_buffer(&device, "blit_u", 16);
        let field_u = uniform_buffer(&device, "field_u", 64);
        let feedback_u = uniform_buffer(&device, "feedback_u", 32);
        let fx_u = uniform_buffer(&device, "fx_u", 48);
        let field_bg = u_bind(&device, &u_bgl, &field_u);

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

        let bufs = FsBufs {
            composite: &composite_u,
            one: &one_u,
            post: &post_u,
            blit: &blit_u,
            fx: &fx_u,
            feedback: &feedback_u,
        };
        let targets = build_targets(&device, &queue, &uts, &samp, &mirror, &bufs, size);

        Self {
            device,
            queue,
            stroke_p,
            point_p,
            prefilter_p,
            down_p,
            up_p,
            composite_p,
            field_p,
            feedback_p,
            fx_p,
            post_p,
            blit_p,
            uts,
            scene_bgl,
            samp,
            mirror,
            scene_u,
            hud_u,
            composite_u,
            one_u,
            post_u,
            blit_u,
            field_u,
            feedback_u,
            fx_u,
            field_bg,
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
            cur: 0,
            trails_were_on: false,
            targets,
        }
    }

    pub fn resize(&mut self, size: (u32, u32)) {
        let bufs = FsBufs {
            composite: &self.composite_u,
            one: &self.one_u,
            post: &self.post_u,
            blit: &self.blit_u,
            fx: &self.fx_u,
            feedback: &self.feedback_u,
        };
        self.targets =
            build_targets(&self.device, &self.queue, &self.uts, &self.samp, &self.mirror, &bufs, size);
        // The trail field is gone with the old textures; don't fade in from it.
        self.trails_were_on = false;
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

    /// Encode P1..P5. The FX stages are skipped entirely when their layer is
    /// off, so the default build encodes exactly the passes it always did.
    pub fn encode(&mut self, encoder: &mut wgpu::CommandEncoder, inputs: &FrameInputs) {
        self.queue.write_buffer(&self.scene_u, 0, bytemuck::bytes_of(&inputs.scene));
        self.queue.write_buffer(&self.hud_u, 0, bytemuck::bytes_of(&inputs.hud));
        self.queue.write_buffer(&self.post_u, 0, bytemuck::bytes_of(&inputs.post));
        // The up-chain accumulates every mip level, so normalize by the level
        // count — otherwise the halo strength scales with resolution.
        let mips = self.targets.bloom_mips.len();
        let comp = CompositeUniforms { gain: inputs.bloom_gain / mips as f32, _pad: [0.0; 3] };
        self.queue.write_buffer(&self.composite_u, 0, bytemuck::bytes_of(&comp));
        // Only the layers that will actually be encoded get a buffer write.
        if inputs.bg_on {
            self.queue.write_buffer(&self.field_u, 0, bytemuck::bytes_of(&inputs.field));
        }
        if inputs.trails_on {
            self.queue.write_buffer(&self.feedback_u, 0, bytemuck::bytes_of(&inputs.feedback));
        }
        if inputs.fg_on {
            self.queue.write_buffer(&self.fx_u, 0, bytemuck::bytes_of(&inputs.fx));
        }

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

        // P3b: trails. history[next] = feedback(history[prev]) + scene.
        if inputs.trails_on {
            if !self.trails_were_on {
                // Switching the layer back on must not fade in from whatever was
                // frozen in the field when it was switched off — quite possibly
                // a different figure entirely.
                for view in &self.targets.history {
                    begin_pass(encoder, "trail-clear", view, true);
                }
            }
            let prev = self.cur;
            let next = 1 - self.cur;
            {
                let mut pass = begin_pass(encoder, "trail", &self.targets.history[next], true);
                pass.set_pipeline(&self.feedback_p);
                pass.set_bind_group(0, &self.targets.feedback_bg[prev], &[]);
                pass.draw(0..3, 0..1);
                // Gain 1, additive: this frame's ink lands in the field at full
                // strength, so the newest line is always exact and only the
                // older ones have decayed.
                pass.set_pipeline(&self.composite_p);
                pass.set_bind_group(0, &self.targets.scene_copy_bg, &[]);
                pass.draw(0..3, 0..1);
            }
            self.cur = next;
        }
        self.trails_were_on = inputs.trails_on;

        // P3c: the background field, added under the ink.
        //
        // It lands *after* the trail pass, and where it lands is load-bearing.
        // The field is near-static, so anything that let it reach the history
        // would accumulate it toward gain/(1 - keep) and wash the frame out
        // within a second of switching trails on — writing it into the
        // ping-pong is not enough, because the feedback reads that same
        // texture next frame.
        //
        // So with trails running, the trail field and the background are summed
        // into a separate compose target that the loop never sees; with trails
        // off there is no loop and the scene itself is the frame. Either way the
        // background contributes exactly once per frame, at the gain asked for,
        // and the foreground still folds it together with the geometry because
        // it reads whichever texture ended up holding the frame.
        //
        // It also lands after the bloom chain — see the shader's header for why
        // a lit fullscreen field must not enter a zero-threshold bloom.
        let composed = inputs.bg_on && inputs.trails_on;
        if composed {
            let mut pass = begin_pass(encoder, "compose", &self.targets.compose_view, true);
            pass.set_pipeline(&self.composite_p);
            pass.set_bind_group(0, &self.targets.hist_copy_bg[self.cur], &[]);
            pass.draw(0..3, 0..1);
            pass.set_pipeline(&self.field_p);
            pass.set_bind_group(0, &self.field_bg, &[]);
            pass.draw(0..3, 0..1);
        } else if inputs.bg_on {
            let mut pass = begin_pass(encoder, "field", &self.targets.scene_view, false);
            pass.set_pipeline(&self.field_p);
            pass.set_bind_group(0, &self.field_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // P4: the foreground remap, reading whichever texture holds the frame.
        if inputs.fg_on {
            let src = if composed {
                &self.targets.fx_bg_compose
            } else if inputs.trails_on {
                &self.targets.fx_bg_hist[self.cur]
            } else {
                &self.targets.fx_bg_scene
            };
            let mut pass = begin_pass(encoder, "fx", &self.targets.fx_view, true);
            pass.set_pipeline(&self.fx_p);
            pass.set_bind_group(0, src, &[]);
            pass.draw(0..3, 0..1);
        }

        // P5: post grade → final_tex.
        {
            let src = if inputs.fg_on {
                &self.targets.post_bg_fx
            } else if composed {
                &self.targets.post_bg_compose
            } else if inputs.trails_on {
                &self.targets.post_bg_hist[self.cur]
            } else {
                &self.targets.post_bg
            };
            let mut pass = begin_pass(encoder, "post", &self.targets.final_view, true);
            pass.set_pipeline(&self.post_p);
            pass.set_bind_group(0, src, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// P6: draw final_tex into the given surface pass.
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
