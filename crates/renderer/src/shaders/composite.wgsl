// silka — compositing a finished layer back into its parent target.
//
// The layer texture is the size of the whole surface and holds absolute
// coordinates, so the UV for a fragment is simply its position divided by the
// viewport. That is what keeps this shader free of any offset arithmetic: the
// quad covers the layer's bounds, and every fragment samples the pixel that was
// drawn at the very same place.
//
// The layer's contents are PREMULTIPLIED (the SDF shader writes premultiplied
// color into a target cleared to transparent), so group opacity is a plain
// multiply and the pipeline blends with One / OneMinusSrcAlpha — the same
// blending as everything else in this backend.

struct Composite {
    // The target size in logical points.
    viewport: vec2<f32>,
    // The layer bounds to composite, in absolute logical points.
    rect_min: vec2<f32>,
    rect_max: vec2<f32>,
    // Group opacity, and padding to keep the uniform a multiple of 16 bytes.
    opacity: f32,
    reserved: f32,
};

@group(0) @binding(0) var<uniform> composite: Composite;
@group(0) @binding(1) var layer: texture_2d<f32>;
@group(0) @binding(2) var layer_sampler: sampler;

struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Varying {
    let corner = vec2<f32>(f32(index & 1u), f32((index >> 1u) & 1u));
    let point = mix(composite.rect_min, composite.rect_max, corner);
    var out: Varying;
    out.position = vec4<f32>(
        point.x / composite.viewport.x * 2.0 - 1.0,
        1.0 - point.y / composite.viewport.y * 2.0,
        0.0,
        1.0,
    );
    // The layer texture spans the whole surface in the same coordinate space, so
    // the UV is the point itself, normalized.
    out.uv = point / composite.viewport;
    return out;
}

@fragment
fn fs_main(in: Varying) -> @location(0) vec4<f32> {
    let texel = textureSampleLevel(layer, layer_sampler, in.uv, 0.0);
    return texel * composite.opacity;
}
