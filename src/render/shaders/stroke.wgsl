// Feathered stroke quad strips, additive.
//
// Two things this shader does that the Colliderscope original didn't:
//
// 1. The reveal is evaluated PER FRAGMENT. Metatron's Cube is 78 straight
//    chords, and a 2-vertex chord under a per-vertex reveal can only produce a
//    linear alpha gradient — no pen head at all. Pushing `u` and the stroke's
//    arc length as varyings gives a pixel-exact leading edge with zero
//    subdivision.
// 2. The view transform lives here, not in the vertex data. Vertices are in
//    construction space, so zoom and pan cost nothing — which is what makes an
//    endlessly growing figure affordable.

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

// Per-step animation state, rewritten every frame (16 bytes each).
struct Step {
    head: f32,           // reveal frontier, 0 = untouched, 1 = fully drawn
    glow: f32,           // base emissive level for this step's role and age
    pen: f32,            // 1 while actively being drawn, else 0
    seed: f32,           // decorrelates this step's shimmer phase
    color: vec3<f32>,    // linear RGB by role (figure / scaffold / accent)
}

@group(0) @binding(0) var<uniform> u: SceneU;
@group(0) @binding(1) var<storage, read> steps: array<Step>;

struct VsIn {
    @location(0) pos: vec2<f32>,
    @location(1) miter: vec2<f32>,
    @location(2) side: f32,
    @location(3) half_width: f32,
    @location(4) u: f32,
    @location(5) arc_len: f32,
    @location(6) energy: f32,
    @location(7) step_id: u32,
}

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) dist: f32,
    @location(1) half_width: f32,
    @location(2) uu: f32,
    @location(3) arc_px: f32,
    @location(4) energy: f32,
    @location(5) @interpolate(flat) sid: u32,
}

const FEATHER: f32 = 1.25;
const PEN_SIGMA: f32 = 9.0;    // px, glint gaussian
const PEN_TAIL: f32 = 55.0;    // px, wake decay behind the pen
const SH_SPEED: f32 = 40.0;    // px/s drift
const SH_LONG: f32 = 34.0;     // px, coarse octave wavelength
const SH_SHORT: f32 = 11.0;    // px, fine octave wavelength

@vertex
fn vs_stroke(in: VsIn) -> VsOut {
    var out: VsOut;
    // Construction (Y-up) → pixels (Y-down). The map is conformal, so the
    // miter vector transforms the same way and stays a unit normal.
    let base = u.view_center + vec2<f32>(in.pos.x, -in.pos.y) * u.view_scale;
    let off = in.half_width + FEATHER;
    let p = base + vec2<f32>(in.miter.x, -in.miter.y) * (off * in.side);

    let ndc = p / u.res * 2.0 - 1.0;
    out.pos = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
    out.dist = off * in.side;
    out.half_width = in.half_width;
    out.uu = in.u;
    // Arc length is stored in construction units so the camera can move freely.
    out.arc_px = in.arc_len * u.view_scale;
    out.energy = in.energy;
    out.sid = in.step_id;
    return out;
}

fn hash1(x: f32) -> f32 {
    return fract(sin(x * 127.1) * 43758.5453);
}

fn vnoise(x: f32) -> f32 {
    let i = floor(x);
    let f = fract(x);
    let w = f * f * (3.0 - 2.0 * f);
    return mix(hash1(i), hash1(i + 1.0), w);
}

@fragment
fn fs_stroke(in: VsOut) -> @location(0) vec4<f32> {
    let st = steps[in.sid];

    // Cross-section coverage.
    let cov = 1.0 - smoothstep(in.half_width - FEATHER, in.half_width + FEATHER, abs(in.dist));

    // Signed distance to the pen, in arc-length PIXELS. d < 0 = already drawn.
    let d = (in.uu - st.head) * in.arc_px;
    // Half-pixel, exactly antialiased reveal edge.
    let reveal = 1.0 - smoothstep(-0.5, 0.5, d);

    // Shimmer: two drifting octaves along arc length. Wavelengths are in pixels
    // so they're resolution-independent, and short enough that several cells sit
    // on every line at once — spatial structure is what makes it read as
    // sparkle rather than the whole line pulsing.
    let s_px = in.uu * in.arc_px;
    let drift = u.time * SH_SPEED;
    let n = vnoise((s_px - drift) / SH_LONG + st.seed) * 0.65
        + vnoise((s_px + drift * 0.37) / SH_SHORT + st.seed * 3.7) * 0.35;
    let shimmer = 1.0 + u.shimmer * (n - 0.5) * 2.0;

    // Pen glint — hot and specular, with a short trailing wake behind the head.
    let glint = 2.5 * exp(-(d * d) / (2.0 * PEN_SIGMA * PEN_SIGMA));
    let wake = 1.1 * exp(min(d, 0.0) / PEN_TAIL);
    let pen = st.pen * (glint + wake);

    // Body carries the palette; the glint stays white — a specular highlight
    // is the colour of the light, not of the body, and that distinction is
    // what keeps a coloured line reading as light rather than paint.
    let k = reveal * cov * in.energy * u.emissive;
    let body = st.color * (st.glow * shimmer * k);
    let glint_w = vec3<f32>(1.0, 1.0, 1.0) * (pen * k);
    return vec4<f32>(body + glint_w, 1.0);
}
