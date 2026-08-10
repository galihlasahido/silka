// silka — the UI-specific SDF shader (REKOMENDASI §3.2, §3.6).
//
// ONE pipeline draws the whole box vocabulary that covers ~95% of a UI:
//
//   * rounded rects with TWO corner geometry modes, selected by PARAMETER
//     rather than by two shaders or a constant:
//       - `exponent` = 2  → an ordinary circular arc (Tailwind/shadcn preset)
//       - `exponent` > 2  → superellipse / squircle (Cupertino preset, HIG
//         continuous corner; Apple's value is ≈ 4)
//   * border: an inset stroke that follows the same corner shape;
//   * drop shadows with a true gaussian blur — used in two layers
//     (ambient + key, HIG style) as two instances;
//   * GLYPHS from an atlas: textured quads that sample alpha coverage and tint
//     it with the run color (a theme token). Because the kind is just a number
//     in `params.w`, text rides in the same draw call as boxes and shadows —
//     order is preserved, so text is never painted over by a background.
//
// THE BINDING IMPELLER LESSON: this file is `include_str!`d when Rust is
// compiled and its module is created once at device initialization. No WGSL is
// assembled, patched, or varied at runtime — every "variant" is instance data,
// which is what keeps frame time predictable.
//
// All coordinates here are LOGICAL POINTS. The anti-aliasing width is derived
// from screen-space derivatives (`fwidth`), so 2x Retina and fractional Wayland
// scales are automatically correct without sending a scale factor to the shader.

// The number of gaussian integration samples for shadows. A build-time constant.
const SHADOW_SAMPLES: i32 = 8;
// sqrt(2*pi) and sqrt(0.5) — used for gaussian normalization and the erf scale.
const SQRT_2PI: f32 = 2.5066283;
const SQRT_HALF: f32 = 0.70710678;
// Boxes are drawn as quads; the kind is distinguished through params.w.
// These are thresholds, not exact values: 0 = box, 1 = shadow, 2 = glyph.
const KIND_SHADOW: f32 = 0.5;
const KIND_GLYPH: f32 = 1.5;
// The atlas selector in params.x for glyph instances.
const ATLAS_COLOR: f32 = 0.5;

struct Globals {
    // The viewport size in logical points.
    viewport: vec2<f32>,
    // Padding to keep the uniform a multiple of 16 bytes.
    reserved: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
// The glyph atlases: alpha coverage for ordinary text, RGBA for color emoji.
// Both are always bound (a 1×1 placeholder while the application has no text)
// so there is no "with/without texture" pipeline variant.
@group(0) @binding(1) var atlas_mask: texture_2d<f32>;
@group(0) @binding(2) var atlas_color: texture_2d<f32>;
@group(0) @binding(3) var atlas_sampler: sampler;

struct Instance {
    // xy = center, zw = half size (logical points).
    @location(0) bounds: vec4<f32>,
    // Per-corner radii: top-left, top-right, bottom-right, bottom-left.
    // Already multiplied by the squircle factor and clamped CPU-side.
    // GLYPH: the same slot carries the UV rect [u0, v0, u1, v1].
    @location(1) radii: vec4<f32>,
    // Fill color (shadow: the shadow color; glyph: the text color from a theme
    // token), straight alpha, in target space.
    @location(2) background: vec4<f32>,
    // Border color, straight alpha. Unused by glyph instances.
    @location(3) border: vec4<f32>,
    // x = border width (glyph: 0 = mask atlas, 1 = color atlas),
    // y = superellipse exponent, z = blur sigma,
    // w = kind (0 = box, 1 = shadow, 2 = glyph).
    @location(4) params: vec4<f32>,
};

struct Varying {
    @builtin(position) position: vec4<f32>,
    // The fragment position relative to the box center, in logical points.
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) half_size: vec2<f32>,
    @location(2) @interpolate(flat) radii: vec4<f32>,
    @location(3) @interpolate(flat) background: vec4<f32>,
    @location(4) @interpolate(flat) border: vec4<f32>,
    @location(5) @interpolate(flat) params: vec4<f32>,
};

// The four triangle-strip points: (-1,-1), (1,-1), (-1,1), (1,1).
fn corner_of(index: u32) -> vec2<f32> {
    return vec2<f32>(f32(index & 1u), f32((index >> 1u) & 1u)) * 2.0 - vec2<f32>(1.0);
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32, inst: Instance) -> Varying {
    let half_size = inst.bounds.zw;
    // The draw margin: 3σ for the shadow's gaussian tail + 1 point for
    // anti-aliasing. Glyphs get NO margin: their box is already exactly the
    // size of the atlas bitmap and already snapped to the physical pixel grid,
    // so one texel lands exactly on one screen pixel — any margin at all would
    // stretch the UVs and make text soft on a 2× display.
    let pad = select(inst.params.z * 3.0 + 1.0, 0.0, inst.params.w > KIND_GLYPH);
    let local = corner_of(index) * (half_size + vec2<f32>(pad));
    let point = inst.bounds.xy + local;

    var out: Varying;
    out.position = vec4<f32>(
        point.x / globals.viewport.x * 2.0 - 1.0,
        1.0 - point.y / globals.viewport.y * 2.0,
        0.0,
        1.0,
    );
    out.local = local;
    out.half_size = half_size;
    out.radii = inst.radii;
    out.background = inst.background;
    out.border = inst.border;
    out.params = inst.params;
    return out;
}

// The corner radius that applies to the quadrant this fragment falls in.
// The y axis points down, matching `silka-paint` coordinates.
fn radius_for(local: vec2<f32>, radii: vec4<f32>) -> f32 {
    let kiri = local.x < 0.0;
    let atas = select(radii.y, radii.x, kiri);
    let bawah = select(radii.z, radii.w, kiri);
    return select(bawah, atas, local.y < 0.0);
}

// The p-norm of a non-negative vector: this is the one and only place where the
// two corner geometry modes differ. n = 2 gives a circle (arc), n > 2 gives a
// superellipse (squircle).
fn norm_p(v: vec2<f32>, n: f32) -> f32 {
    if (n <= 2.0) {
        return length(v);
    }
    return pow(pow(v.x, n) + pow(v.y, n), 1.0 / n);
}

// The magnitude of the p-norm's gradient. For n > 2 the field is not a true
// distance (its gradient shrinks toward the diagonal), so the anti-aliasing band
// would widen at the corners if it were not normalized.
fn norm_p_gradient(v: vec2<f32>, n: f32) -> f32 {
    if (n <= 2.0) {
        return 1.0;
    }
    let d = norm_p(v, n);
    if (d <= 1e-6) {
        return 1.0;
    }
    let gx = pow(v.x / d, n - 1.0);
    let gy = pow(v.y / d, n - 1.0);
    return max(sqrt(gx * gx + gy * gy), 1e-4);
}

// The signed distance to a rounded/squircle box.
// Negative inside, positive outside, in logical points.
fn sd_shape(local: vec2<f32>, half_size: vec2<f32>, radii: vec4<f32>, n: f32) -> f32 {
    let r = radius_for(local, radii);
    let q = abs(local) - half_size + vec2<f32>(r);
    let luar = max(q, vec2<f32>(0.0));
    let dalam = min(max(q.x, q.y), 0.0);
    return (dalam + norm_p(luar, n) - r) / norm_p_gradient(luar, n);
}

// Pixel coverage from a signed distance: a band one screen pixel wide.
fn coverage(sd: f32, px: f32) -> f32 {
    return clamp(0.5 - sd / px, 0.0, 1.0);
}

fn premultiply(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(c.rgb * c.a, c.a);
}

fn gaussian(x: f32, sigma: f32) -> f32 {
    let t = x / sigma;
    return exp(-0.5 * t * t) / (sigma * SQRT_2PI);
}

// An erf approximation (Abramowitz & Stegun 7.1.27), two values at once.
fn erf2(v: vec2<f32>) -> vec2<f32> {
    let s = sign(v);
    let a = abs(v);
    var r = 1.0 + (0.278393 + (0.230389 + 0.078108 * a * a) * a) * a;
    r = r * r;
    r = r * r;
    return s - s / r;
}

// The shape's half width at row `y` (relative to the center). For arc corners
// this is sqrt(r² - dy²); for squircles it uses the same p-norm as the SDF, so
// the shadow of a squircle card is itself a squircle.
fn half_width_at(y: f32, half_size: vec2<f32>, r: f32, n: f32) -> f32 {
    let dy = clamp(abs(y) - (half_size.y - r), 0.0, r);
    var qx = 0.0;
    if (r > 0.0) {
        if (n <= 2.0) {
            qx = sqrt(max(r * r - dy * dy, 0.0));
        } else {
            qx = pow(max(pow(r, n) - pow(dy, n), 0.0), 1.0 / n);
        }
    }
    return max(half_size.x - r + qx, 0.0);
}

// A gaussian-blurred rounded box, Evan Wallace style: analytic integration
// (erf) along x, numeric along y. Far cheaper than a two-pass blur through a
// texture, and that is what makes HIG-style double shadows free at 120 fps.
fn shadow_coverage(local: vec2<f32>, half_size: vec2<f32>, r: f32, n: f32, sigma: f32) -> f32 {
    let s = max(sigma, 1e-3);
    let low = local.y - half_size.y;
    let high = local.y + half_size.y;
    let start = clamp(-3.0 * s, low, high);
    let end = clamp(3.0 * s, low, high);
    let step = (end - start) / f32(SHADOW_SAMPLES);
    var y = start + step * 0.5;
    var total = 0.0;
    for (var i = 0; i < SHADOW_SAMPLES; i = i + 1) {
        let hw = half_width_at(local.y - y, half_size, r, n);
        let e = erf2((vec2<f32>(local.x) + vec2<f32>(-hw, hw)) * (SQRT_HALF / s));
        total = total + 0.5 * (e.y - e.x) * gaussian(y, s) * step;
        y = y + step;
    }
    return clamp(total, 0.0, 1.0);
}

@fragment
fn fs_main(in: Varying) -> @location(0) vec4<f32> {
    let n = in.params.y;
    let sd = sd_shape(in.local, in.half_size, in.radii, n);
    // `fwidth` must be called in uniform control flow — which is why it lives
    // here, before the instance kind branch.
    let px = max(fwidth(sd), 1e-5);

    var warna: vec4<f32>;
    if (in.params.w > KIND_GLYPH) {
        // Destination box → UV rect, both axis-aligned: the mapping is affine.
        // `textureSampleLevel` (not `textureSample`) keeps sampling legal
        // inside a branch — and the atlas has no mips anyway.
        let t = clamp(in.local / max(in.half_size, vec2<f32>(1e-6)) * 0.5 + vec2<f32>(0.5),
                      vec2<f32>(0.0), vec2<f32>(1.0));
        let uv = mix(in.radii.xy, in.radii.zw, t);
        if (in.params.x > ATLAS_COLOR) {
            // Emoji: the color comes from the atlas, but the run's alpha is
            // still honored so fade/disabled work the same as for plain text.
            let texel = textureSampleLevel(atlas_color, atlas_sampler, uv, 0.0);
            warna = premultiply(texel) * in.background.a;
        } else {
            // Plain text: the atlas stores only COVERAGE, the color comes from
            // a theme token through the instance — which is why one "a" bitmap
            // serves label, secondary label, and accent all at once.
            let cakupan = textureSampleLevel(atlas_mask, atlas_sampler, uv, 0.0).r;
            warna = premultiply(in.background) * cakupan;
        }
    } else if (in.params.w > KIND_SHADOW) {
        // The gaussian blur uses a single radius; differences between corners
        // are invisible once blurred, so the average is used.
        let r = dot(in.radii, vec4<f32>(0.25));
        warna = premultiply(in.background) * shadow_coverage(
            in.local,
            in.half_size,
            r,
            n,
            in.params.z,
        );
    } else {
        let luar = coverage(sd, px);
        // The border is the ring between the outer edge and an edge inset by
        // the border width — it automatically follows squircle or arc alike.
        let dalam = coverage(sd + in.params.x, px);
        warna = premultiply(in.background) * dalam
            + premultiply(in.border) * max(luar - dalam, 0.0);
    }
    return warna;
}
