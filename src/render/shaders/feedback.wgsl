// Ghost trails: fused decay + rotate/zoom self-blit.
//
//   out = keep · ( prev(uv) + flow_alpha · prev(T⁻¹ uv) ),  clamped
//
// The second term is what separates this from a plain fade. Sampling the
// previous field through an inverse rotate/zoom means every echo lands slightly
// turned and slightly larger than the one before it, so a still figure grows a
// receding spiral of itself — the feedback tunnel — rather than a symmetric blur.
//
// Every constant arrives already framerate-corrected from the CPU (see
// `Fx::feedback_uniforms`), so trail length is a wall-clock property, not a
// frame-count one.
//
// `clamp_max` is the loop's safety rail. The other one is structural: bloom runs
// over the scene texture only and never over this field, so the loop carries no
// bloom gain and cannot compound its own halo.

struct FeedbackUniforms {
    keep: f32,
    flow_alpha: f32,
    rot: f32,
    inv_scale: f32,
    clamp_max: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

@group(0) @binding(0) var<uniform> u: FeedbackUniforms;
@group(0) @binding(1) var prev_tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

@fragment
fn fs_feedback(in: FsOut) -> @location(0) vec4<f32> {
    let base = textureSampleLevel(prev_tex, samp, in.uv, 0.0).rgb;

    let c = in.uv - vec2<f32>(0.5);
    let cs = cos(-u.rot);
    let sn = sin(-u.rot);
    let q = vec2<f32>(c.x * cs - c.y * sn, c.x * sn + c.y * cs) * u.inv_scale + vec2<f32>(0.5);
    let flow = textureSampleLevel(prev_tex, samp, q, 0.0).rgb;

    let outc = min(u.keep * (base + u.flow_alpha * flow), vec3<f32>(u.clamp_max));
    return vec4<f32>(outc, 1.0);
}
