// Final grade: bright → tonemap → sRGB encode → vignette.
// Writes sRGB-encoded bytes into an Rgba8Unorm target.
//
// No hue rotate, no saturation, no posterize, no flash — this app is white on
// black, and a colour grade would only fight that.

struct PostUniforms {
    res: vec2<f32>,
    bright: f32,
    vignette: f32,
}

@group(0) @binding(0) var<uniform> u: PostUniforms;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

// Identity below the knee, C1-smooth rational rolloff to 1 above it.
// The knee is load-bearing for shimmer: crests that cross it compress in the
// core while their (pre-tonemap) bloom halo keeps growing, which is what makes
// the modulation read as specular sparkle instead of brightness flicker.
fn tonemap_ch(x: f32) -> f32 {
    let k = 0.8;
    if x <= k {
        return x;
    }
    let t = x - k;
    return k + (1.0 - k) * t / (t + (1.0 - k));
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        return c * 12.92;
    }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

@fragment
fn fs_post(in: FsOut) -> @location(0) vec4<f32> {
    var col = textureSampleLevel(src, samp, in.uv, 0.0).rgb;
    col = max(col * u.bright, vec3<f32>(0.0));

    col = vec3<f32>(tonemap_ch(col.r), tonemap_ch(col.g), tonemap_ch(col.b));
    col = vec3<f32>(linear_to_srgb(col.r), linear_to_srgb(col.g), linear_to_srgb(col.b));

    let aspect = u.res.x / u.res.y;
    var uv0 = in.uv - vec2<f32>(0.5);
    uv0.x *= aspect;
    col *= 1.0 - u.vignette * pow(min(length(uv0), 1.2), 2.4);

    return vec4<f32>(col, 1.0);
}
