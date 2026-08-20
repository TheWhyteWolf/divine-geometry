// Background layer: a procedural field, generated from nothing and added under
// the ink.
//
// It binds no source texture — it is a pure function of position and time — and
// it is composited *after* the bloom chain. That ordering is deliberate: the
// bloom threshold is zero on purpose so the faintest scaffold line still haloes,
// and a lit fullscreen field entering the chain at threshold zero would bloom
// the whole frame into a wash. Added afterwards it stays a base layer, and
// because the renderer is additive over black, "behind" is a brightness
// relationship — the ink is always the brighter thing.
//
// The field takes its hue from the live palette, so it sits inside the figure's
// colour scheme instead of arguing with it; `sat` opens that single hue out into
// a spread. The centre is held back so the construction always has calm ground
// to be legible against.
//
// The two fractal modes are ports of the Colliderscope PRISM shader's escape-time
// functions.

struct FieldUniforms {
    res: vec2<f32>,
    time: f32,
    gain: f32,
    /// Palette accent, linear RGB — tints the achromatic modes.
    tint: vec3<f32>,
    /// Spatial frequency of the field. Larger = finer.
    scale: f32,
    /// Palette hue, degrees.
    hue: f32,
    /// Hue spread. 0 = the whole field is the palette's one hue.
    sat: f32,
    /// Mirror count for the folding mode.
    seg: f32,
    warp: f32,
    frac_c: vec2<f32>,
    frac_iter: f32,
    mode: u32,
}

@group(0) @binding(0) var<uniform> u: FieldUniforms;

const PI: f32 = 3.14159265;
const TAU: f32 = 6.2831853;
const MAXI: u32 = 48u;

fn fmod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

fn fmod3(x: vec3<f32>, y: f32) -> vec3<f32> {
    return x - y * floor(x / y);
}

fn wedge(a: f32) -> f32 {
    let seg = TAU / max(u.seg, 1.0);
    return abs(fmod(a, seg) - seg * 0.5);
}

fn hsl(h: f32, s: f32, l: f32) -> vec3<f32> {
    let r = clamp(
        abs(fmod3(h * 6.0 + vec3<f32>(0.0, 4.0, 2.0), 6.0) - 3.0) - 1.0,
        vec3<f32>(0.0),
        vec3<f32>(1.0),
    );
    return l + s * (r - 0.5) * (1.0 - abs(2.0 * l - 1.0));
}

/// The palettes here are authored in gamma space; the pipeline is linear.
fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2));
}

/// Hoskins' sine-free hash.
///
/// The usual `fract(sin(dot(p, k)) * 43758.5)` is not usable here. The domain
/// warp feeds noise its own output as coordinates, twice, which pushes the
/// argument of that `sin` far enough out that f32 loses the low bits — and the
/// hash degenerates into visible rectangular blocks. Mixing in fract space has
/// no such cliff.
fn hash2(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, vec3<f32>(p3.y, p3.z, p3.x) + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn vnoise2(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let w = f * f * (3.0 - 2.0 * f);
    let a = hash2(i);
    let b = hash2(i + vec2<f32>(1.0, 0.0));
    let c = hash2(i + vec2<f32>(0.0, 1.0));
    let d = hash2(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, w.x), mix(c, d, w.x), w.y);
}

fn fbm(p0: vec2<f32>) -> f32 {
    var p = p0;
    var amp = 0.5;
    var sum = 0.0;
    for (var i = 0u; i < 5u; i++) {
        sum += amp * vnoise2(p);
        p = p * 2.03 + vec2<f32>(1.7, 9.2);
        amp *= 0.5;
    }
    return sum;
}

fn f_kali(p0: vec2<f32>, it: f32, c: vec2<f32>) -> f32 {
    var p = p0;
    var trap = 1e9;
    for (var i = 0u; i < MAXI; i++) {
        if f32(i) >= it { break; }
        p = abs(p) / max(dot(p, p), 1e-6) - c;
        trap = min(trap, length(p));
    }
    return 1.0 / (1.0 + trap * 6.0);
}

fn f_julia(z0: vec2<f32>, c: vec2<f32>, it: f32) -> f32 {
    var z = z0;
    var n = 0.0;
    for (var i = 0u; i < MAXI; i++) {
        if f32(i) >= it { break; }
        z = vec2<f32>(z.x * z.x - z.y * z.y, 2.0 * z.x * z.y) + c;
        if dot(z, z) > 16.0 {
            n = f32(i) - log2(log2(max(dot(z, z), 2.0))) + 4.0;
            break;
        }
        n = f32(i);
    }
    return n / max(it, 1.0);
}

@fragment
fn fs_field(in: FsOut) -> @location(0) vec4<f32> {
    let aspect = u.res.x / u.res.y;
    var p = in.uv - vec2<f32>(0.5);
    p.x *= aspect;

    let r = max(length(p), 1e-6);
    let ang = atan2(p.y, p.x);
    let t = u.time;
    let q = p * u.scale;

    // Field value in 0..1, and the hue offset it drives.
    var v = 0.0;
    var f = 0.0;

    switch u.mode {
        case 1u: {
            // Plasma — iq's domain warp: noise whose *coordinates* are noise,
            // twice over. Produces the slow marbling that never repeats.
            let a = vec2<f32>(fbm(q + vec2<f32>(0.0, t * 0.15)), fbm(q + vec2<f32>(5.2, 1.3) - t * 0.11));
            let b = vec2<f32>(
                fbm(q + 3.0 * a + vec2<f32>(1.7, 9.2) - t * 0.09),
                fbm(q + 3.0 * a + vec2<f32>(8.3, 2.8) + t * 0.13),
            );
            f = fbm(q + 3.0 * b);
            v = smoothstep(0.25, 0.95, f);
        }
        case 2u: {
            // Tunnel — a bore receding to a vanishing point at the centre,
            // ribbed across and scrolling down its length.
            let uu = ang / TAU;
            let vv = 0.4 / (r + 0.03) + t * 0.35;
            let ribs = 0.5 + 0.5 * sin(vv * TAU);
            let staves = 0.5 + 0.5 * sin(uu * TAU * max(u.seg, 1.0));
            f = fract(vv * 0.25);
            v = ribs * (0.35 + 0.65 * staves) * smoothstep(0.0, 0.12, r);
        }
        case 3u: {
            // Kaleido field — the wedge fold applied to concentric rings, so the
            // ground carries the same rotational symmetry as the figure on it.
            let a = wedge(ang + t * 0.13);
            let rings = 0.5 + 0.5 * cos(r * (6.0 * u.scale) - t * 1.1 + a * 6.0);
            let petals = 0.5 + 0.5 * cos(a * 8.0 - t * 0.4);
            f = rings * petals;
            v = pow(f, 1.6);
        }
        case 4u: {
            // Kali — the fold-and-invert orbit trap. Filigree, not blobs.
            let a = wedge(ang + t * 0.05);
            let fp = vec2<f32>(cos(a), sin(a)) * r * (2.4 * u.scale) + u.frac_c * 0.1;
            f = f_kali(fp, u.frac_iter, u.frac_c);
            v = pow(f, 1.3);
        }
        default: {
            // Julia — escape time, with the seed drifting so the set breathes.
            let c = u.frac_c + vec2<f32>(sin(t * 0.11), cos(t * 0.09)) * 0.06;
            f = f_julia(q * 1.4, c, u.frac_iter);
            v = smoothstep(0.05, 0.75, f);
        }
    }

    // Warp is a soft brightness ripple here rather than a coordinate push — the
    // background has no image to distort, only a value to modulate.
    v *= 1.0 + u.warp * 0.35 * sin(r * 9.0 - t * 1.3 + f * 4.0);

    // Hold the middle back so the construction always has calm ground.
    v *= mix(0.22, 1.0, smoothstep(0.05, 0.62, r));

    // One hue at sat = 0, opening into a spread as it rises.
    let h = fract(u.hue / 360.0 + u.sat * (f * 0.6 + r * 0.15) + t * 0.013);
    let chroma = clamp(u.sat * 1.4, 0.0, 1.0);
    let col = mix(u.tint, to_linear(hsl(h, 1.0, 0.55)), chroma) * max(v, 0.0);

    return vec4<f32>(col * u.gain, 1.0);
}
