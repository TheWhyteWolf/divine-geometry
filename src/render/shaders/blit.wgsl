// final_tex (sRGB bytes) → surface. If the surface format is sRGB, the shader
// decodes so the hardware re-encode is an identity round trip.

struct BlitUniforms {
    decode_srgb: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<uniform> u: BlitUniforms;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

@fragment
fn fs_blit(in: FsOut) -> @location(0) vec4<f32> {
    var c = textureSampleLevel(src, samp, in.uv, 0.0).rgb;
    if u.decode_srgb == 1u {
        c = vec3<f32>(srgb_to_linear(c.r), srgb_to_linear(c.g), srgb_to_linear(c.b));
    }
    return vec4<f32>(c, 1.0);
}
