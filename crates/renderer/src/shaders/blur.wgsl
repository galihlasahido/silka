// silka — dual-Kawase blur, the first effect that runs on a finished layer.
//
// Why this filter and not a gaussian: a separable gaussian wide enough for a
// material (σ ≈ 20 px) needs dozens of taps per pixel at full resolution. The
// dual-Kawase down/up chain reaches the same visual radius by halving the
// resolution each step — 5 and 8 taps per pixel on progressively smaller
// textures — and the difference from a true gaussian is invisible behind
// translucent UI. It is what the compositors on all three platforms use.
//
// The chain is driven from the Rust side (`crate::layer`): N down passes, then N
// up passes back to the layer's own texture. Every "variant" is the number of
// passes, so — as everywhere else in this backend — no WGSL is assembled at
// runtime.
//
// Texel size comes from `textureDimensions`, so neither pass needs a uniform:
// one bind group per source texture is the whole state.

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var src_sampler: sampler;

struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// A full-target triangle strip: (0,0), (1,0), (0,1), (1,1) in UV space.
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Varying {
    let uv = vec2<f32>(f32(index & 1u), f32((index >> 1u) & 1u));
    var out: Varying;
    out.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

fn texel_size() -> vec2<f32> {
    let dim = vec2<f32>(textureDimensions(src, 0));
    return 1.0 / max(dim, vec2<f32>(1.0));
}

// Downsample: 5 taps (centre + four diagonals), the centre weighted double.
@fragment
fn fs_down(in: Varying) -> @location(0) vec4<f32> {
    let t = texel_size();
    var sum = textureSampleLevel(src, src_sampler, in.uv, 0.0) * 4.0;
    sum = sum + textureSampleLevel(src, src_sampler, in.uv + vec2<f32>(-t.x, -t.y), 0.0);
    sum = sum + textureSampleLevel(src, src_sampler, in.uv + vec2<f32>(t.x, -t.y), 0.0);
    sum = sum + textureSampleLevel(src, src_sampler, in.uv + vec2<f32>(-t.x, t.y), 0.0);
    sum = sum + textureSampleLevel(src, src_sampler, in.uv + vec2<f32>(t.x, t.y), 0.0);
    return sum / 8.0;
}

// Upsample: 8 taps in a rotated tent — the half of the filter that removes the
// blockiness a plain bilinear upscale would leave behind.
@fragment
fn fs_up(in: Varying) -> @location(0) vec4<f32> {
    let t = texel_size();
    var sum = textureSampleLevel(src, src_sampler, in.uv + vec2<f32>(-t.x * 2.0, 0.0), 0.0);
    sum = sum + textureSampleLevel(src, src_sampler, in.uv + vec2<f32>(-t.x, t.y), 0.0) * 2.0;
    sum = sum + textureSampleLevel(src, src_sampler, in.uv + vec2<f32>(0.0, t.y * 2.0), 0.0);
    sum = sum + textureSampleLevel(src, src_sampler, in.uv + vec2<f32>(t.x, t.y), 0.0) * 2.0;
    sum = sum + textureSampleLevel(src, src_sampler, in.uv + vec2<f32>(t.x * 2.0, 0.0), 0.0);
    sum = sum + textureSampleLevel(src, src_sampler, in.uv + vec2<f32>(t.x, -t.y), 0.0) * 2.0;
    sum = sum + textureSampleLevel(src, src_sampler, in.uv + vec2<f32>(0.0, -t.y * 2.0), 0.0);
    sum = sum + textureSampleLevel(src, src_sampler, in.uv + vec2<f32>(-t.x, -t.y), 0.0) * 2.0;
    return sum / 12.0;
}
