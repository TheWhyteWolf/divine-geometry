// Mip-chain bloom (CoD/Jimenez style): prefilter with soft-knee threshold +
// Karis average, 13-tap downsample, 9-tap tent upsample (additive via blend state).
//
// Threshold is 0 here: on a black field with white lines, thresholding would kill
// the halo on the faint scaffold — which is exactly the halo we want.

struct BloomUniforms {
    texel: vec2<f32>,     // 1 / source mip size
    threshold: f32,
    knee: f32,
}

@group(0) @binding(0) var<uniform> u: BloomUniforms;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

fn s(uv: vec2<f32>, dx: f32, dy: f32) -> vec3<f32> {
    return textureSampleLevel(src, samp, uv + vec2<f32>(dx, dy) * u.texel, 0.0).rgb;
}

// Soft-knee threshold curve.
fn prefilter(c: vec3<f32>) -> vec3<f32> {
    let br = max(c.r, max(c.g, c.b));
    let soft = clamp(br - u.threshold + u.knee, 0.0, 2.0 * u.knee);
    let soft2 = soft * soft / (4.0 * u.knee + 1e-5);
    let contrib = max(soft2, br - u.threshold) / max(br, 1e-5);
    return c * max(contrib, 0.0);
}

fn karis(c: vec3<f32>) -> f32 {
    return 1.0 / (1.0 + max(c.r, max(c.g, c.b)));
}

// 13-tap Jimenez downsample. `first` applies threshold + Karis-weighted groups.
fn down13(uv: vec2<f32>, first: bool) -> vec3<f32> {
    let a = s(uv, -2.0, -2.0); let b = s(uv, 0.0, -2.0); let c = s(uv, 2.0, -2.0);
    let d = s(uv, -1.0, -1.0); let e = s(uv, 1.0, -1.0);
    let f = s(uv, -2.0, 0.0);  let g = s(uv, 0.0, 0.0);  let h = s(uv, 2.0, 0.0);
    let i = s(uv, -1.0, 1.0);  let j = s(uv, 1.0, 1.0);
    let k = s(uv, -2.0, 2.0);  let l = s(uv, 0.0, 2.0);  let m = s(uv, 2.0, 2.0);

    if first {
        // Per-group Karis average kills single-pixel HDR fireflies.
        var g0 = (d + e + i + j) * 0.25; // center group, weight 0.5
        var g1 = (a + b + f + g) * 0.25;
        var g2 = (b + c + g + h) * 0.25;
        var g3 = (f + g + k + l) * 0.25;
        var g4 = (g + h + l + m) * 0.25;
        g0 = prefilter(g0) * karis(g0);
        g1 = prefilter(g1) * karis(g1);
        g2 = prefilter(g2) * karis(g2);
        g3 = prefilter(g3) * karis(g3);
        g4 = prefilter(g4) * karis(g4);
        return g0 * 0.5 + (g1 + g2 + g3 + g4) * 0.125;
    }
    let center = (d + e + i + j) * 0.25 * 0.5;
    let corners = ((a + b + f + g) + (b + c + g + h) + (f + g + k + l) + (g + h + l + m)) * 0.25 * 0.125;
    return center + corners;
}

@fragment
fn fs_prefilter(in: FsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(down13(in.uv, true), 1.0);
}

@fragment
fn fs_down(in: FsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(down13(in.uv, false), 1.0);
}

// 9-tap tent upsample; blend state adds it into the destination mip.
@fragment
fn fs_up(in: FsOut) -> @location(0) vec4<f32> {
    let c = s(in.uv, -1.0, -1.0) + s(in.uv, 0.0, -1.0) * 2.0 + s(in.uv, 1.0, -1.0)
        + s(in.uv, -1.0, 0.0) * 2.0 + s(in.uv, 0.0, 0.0) * 4.0 + s(in.uv, 1.0, 0.0) * 2.0
        + s(in.uv, -1.0, 1.0) + s(in.uv, 0.0, 1.0) * 2.0 + s(in.uv, 1.0, 1.0);
    return vec4<f32>(c / 16.0, 1.0);
}
