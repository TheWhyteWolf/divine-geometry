// Foreground layer: a screen-space remap of the composed image.
//
// This pass never invents light — it only decides *where* each pixel reads from.
// Everything it can do is a change of coordinates on the already-lit frame plus
// a chromatic split on the fetch, which is why a kaleidoscope here mirrors the
// figure and the background field together as one image (the layer model the
// pipeline commits to) instead of folding a foreground sprite over the top.
//
// Ported from the Colliderscope kaleido pass, minus its fractal tail — fractals
// belong to the background generator in this app, not to the fold.
//
// Two port notes carried over because both are easy to get wrong:
//   * GLSL `mod()` is floor-mod, WGSL `%` is trunc. `wedge()` needs floor-mod or
//     a seam opens along the -x axis where the angle goes negative.
//   * The edge behaviour of a kaleidoscope IS mirror-repeat wrapping. This pass
//     is bound with a MirrorRepeat sampler; the rest of the pipeline uses the
//     clamp sampler.

struct FxUniforms {
    res: vec2<f32>,
    time: f32,
    /// Master intensity. Rides swirl, warp and chroma together so one knob takes
    /// the whole layer from a hint to a full trip.
    amount: f32,
    /// Mirror count for the folding modes.
    seg: f32,
    rot: f32,
    zoom: f32,
    swirl: f32,
    warp: f32,
    chroma: f32,
    /// Tunnel depth scroll, in screens per second.
    scroll: f32,
    mode: u32,
}

@group(0) @binding(0) var<uniform> u: FxUniforms;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;   // MirrorRepeat + linear

const PI: f32 = 3.14159265;
const TAU: f32 = 6.2831853;

fn fmod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

/// Fold an angle into one mirrored wedge of `seg` segments.
fn wedge(a: f32) -> f32 {
    let seg = TAU / max(u.seg, 1.0);
    return abs(fmod(a, seg) - seg * 0.5);
}

/// Two octaves of a drifting sinusoidal flow field — the "simple distortion".
fn warp_field(p: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(sin(p.y * 3.0 + u.time * 0.7), cos(p.x * 3.0 + u.time * 0.6)) * 0.08
        + vec2<f32>(sin(p.y * 6.3 - u.time * 0.9), cos(p.x * 6.3 + u.time * 1.1)) * 0.035;
}

@fragment
fn fs_fx(in: FsOut) -> @location(0) vec4<f32> {
    let aspect = u.res.x / u.res.y;
    var uv = in.uv - vec2<f32>(0.5);
    uv.x *= aspect;
    uv += warp_field(uv) * (u.warp * u.amount);

    let r = max(length(uv), 1e-6);
    var ang = atan2(uv.y, uv.x) + u.rot;
    ang += sin(r * 5.0 - u.time * 0.9) * (u.swirl * u.amount);

    var suv: vec2<f32>;
    var fog = 1.0;
    switch u.mode {
        case 1u: {
            // Warp — no fold. A rotating view with a radial ripple travelling
            // outward through it; the geometry stays recognisable and breathes.
            let cs = cos(u.rot);
            let sn = sin(u.rot);
            var p = vec2<f32>(uv.x * cs - uv.y * sn, uv.x * sn + uv.y * cs) * u.zoom;
            p += (uv / r) * sin(r * 14.0 - u.time * 2.0) * 0.03 * u.amount;
            p.x /= aspect;
            suv = p + vec2<f32>(0.5);
        }
        case 2u: {
            // Kaleidoscope — the classical radial wedge mirror.
            let a = wedge(ang);
            var p = vec2<f32>(cos(a), sin(a)) * r * u.zoom;
            p.x /= aspect;
            suv = p + vec2<f32>(0.5);
        }
        case 3u: {
            // Tunnel — angle across, inverse radius down the bore, scrolling.
            let a = wedge(ang);
            let uu = a / (PI / max(u.seg, 1.0));
            let vv = 0.45 / (r * u.zoom + 0.04) + u.time * u.scroll;
            suv = vec2<f32>(uu, vv);
            // Depth fog. Near the vanishing point one texel of output covers an
            // unbounded stretch of input, and a level-0 sample there is pure
            // aliasing — sparkling grit, not distance. Fading it out reads as
            // depth and costs nothing.
            fog = smoothstep(0.0, 0.16, r);
        }
        case 4u: {
            // Droste — the wedge fold twisted by log(r), so the image spirals
            // into itself and every turn is a scaled copy of the last.
            let a = wedge(ang + log(r + 0.001) * 1.3);
            var p = vec2<f32>(cos(a), sin(a)) * r * u.zoom;
            p.x /= aspect;
            suv = p + vec2<f32>(0.5);
        }
        default: {
            // Vortex — wedge mirror with a radius-dependent twist.
            let a = wedge(ang);
            let tw = r * 5.0;
            var p = vec2<f32>(cos(a + tw), sin(a + tw)) * r * u.zoom;
            p.x /= aspect;
            suv = p + vec2<f32>(0.5);
        }
    }

    // Chromatic split on the fetch: the three channels read the image at
    // slightly different magnifications, so edges fringe the way they do
    // through a cheap lens.
    let ca = u.chroma * u.amount * 0.03;
    var col: vec3<f32>;
    col.r = textureSampleLevel(src, samp, (suv - 0.5) * (1.0 + ca) + 0.5, 0.0).r;
    col.g = textureSampleLevel(src, samp, suv, 0.0).g;
    col.b = textureSampleLevel(src, samp, (suv - 0.5) * (1.0 - ca) + 0.5, 0.0).b;

    return vec4<f32>(max(col * fog, vec3<f32>(0.0)), 1.0);
}
