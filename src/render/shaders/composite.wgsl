// Scaled textured blit — composites the bloom chain back into the scene
// (additive blend state).

struct CompositeUniforms {
    gain: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

@group(0) @binding(0) var<uniform> u: CompositeUniforms;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

@fragment
fn fs_composite(in: FsOut) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(src, samp, in.uv, 0.0).rgb * u.gain;
    return vec4<f32>(c, 1.0);
}
