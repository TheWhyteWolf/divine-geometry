// Instanced point sprites with gaussian falloff, additive. These are the
// construction nodes — the solved intersections. Intensity is a scalar; the
// birth flash is computed CPU-side and arrives in `level`.

struct SceneU {
    res: vec2<f32>,
    emissive: f32,
    time: f32,
    tint: vec3<f32>,
    shimmer: f32,
    view_center: vec2<f32>,
    view_scale: f32,
    _pad: f32,
}

@group(0) @binding(0) var<uniform> u: SceneU;

struct Inst {
    @location(0) center: vec2<f32>,
    @location(1) radius: f32,
    @location(2) level: f32,
    @location(3) seed: f32,
}

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) radius: f32,
    @location(2) level: f32,
    @location(3) seed: f32,
}

@vertex
fn vs_point(@builtin(vertex_index) vi: u32, inst: Inst) -> VsOut {
    var out: VsOut;
    let corner = vec2<f32>(
        f32(vi & 1u) * 2.0 - 1.0,
        f32((vi >> 1u) & 1u) * 2.0 - 1.0,
    );
    // Node radius is in pixels — a solved point is a mark on the page, not a
    // feature of the figure, so it must not grow when the camera zooms in.
    let ext = inst.radius + 2.5;
    let base = u.view_center + vec2<f32>(inst.center.x, -inst.center.y) * u.view_scale;
    let p = base + corner * ext;
    let ndc = p / u.res * 2.0 - 1.0;
    out.pos = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
    out.local = corner * ext;
    out.radius = inst.radius;
    out.level = inst.level;
    out.seed = inst.seed;
    return out;
}

@fragment
fn fs_point(in: VsOut) -> @location(0) vec4<f32> {
    let d = length(in.local) / max(in.radius, 0.001);
    let fall = exp(-4.0 * d * d) * step(d, 2.5);
    // Slow independent twinkle, phase-decorrelated per node by `seed`.
    let tw = 1.0 + u.shimmer * 0.45 * sin(u.time * 1.7 + in.seed * 6.2831853);
    let v = in.level * tw * fall * u.emissive;
    return vec4<f32>(u.tint * v, 1.0);
}
