// rustui — shader SDF khusus UI (REKOMENDASI §3.2, §3.6).
//
// SATU pipeline menggambar seluruh kosakata kotak yang menutupi ~95% UI:
//
//   * rounded rect dengan DUA mode geometri sudut lewat PARAMETER, bukan dua
//     shader dan bukan konstanta:
//       - `exponent` = 2  → busur lingkaran biasa (preset Tailwind/shadcn)
//       - `exponent` > 2  → superellipse / squircle (preset Cupertino, HIG
//         continuous corner; nilai Apple ≈ 4)
//   * border: stroke di dalam tepi, ikut bentuk sudut yang sama;
//   * drop shadow ber-blur gaussian sejati — dipakai berlapis dua
//     (ambient + key ala HIG) sebagai dua instance;
//   * GLYPH dari atlas: quad bertekstur yang men-sample cakupan alpha lalu
//     mewarnainya dengan warna run (token theme). Karena jenisnya cuma sebuah
//     angka di `params.w`, teks ikut dalam draw call yang sama dengan kotak
//     dan bayangan — urutannya terjaga, jadi teks tidak pernah tertimpa latar.
//
// PELAJARAN IMPELLER YANG MENGIKAT: file ini di-`include_str!` saat kompilasi
// Rust dan modulnya dibuat sekali di inisialisasi device. Tidak ada WGSL yang
// dirakit, ditambal, atau divariasikan saat runtime — semua "varian" adalah
// data instance, sehingga frame time tetap prediktabel.
//
// Semua koordinat di sini adalah POIN LOGIS. Lebar anti-alias diturunkan dari
// derivatif layar (`fwidth`), jadi Retina 2x maupun scale pecahan Wayland
// otomatis benar tanpa mengirim scale factor ke shader.

// Jumlah cuplikan integrasi gaussian untuk shadow. Konstanta build-time.
const SHADOW_SAMPLES: i32 = 8;
// sqrt(2*pi) dan sqrt(0.5) — dipakai normalisasi gaussian dan skala erf.
const SQRT_2PI: f32 = 2.5066283;
const SQRT_HALF: f32 = 0.70710678;
// Kotak digambar sebagai quad; jenisnya dibedakan lewat params.w.
// Ambang, bukan nilai persis: 0 = kotak, 1 = bayangan, 2 = glyph.
const KIND_SHADOW: f32 = 0.5;
const KIND_GLYPH: f32 = 1.5;
// Pemilih atlas di params.x untuk instance glyph.
const ATLAS_COLOR: f32 = 0.5;

struct Globals {
    // Ukuran viewport dalam poin logis.
    viewport: vec2<f32>,
    // Padding agar uniform tetap kelipatan 16 byte.
    reserved: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
// Atlas glyph: cakupan alpha untuk teks biasa, RGBA untuk emoji berwarna.
// Keduanya selalu ter-bind (placeholder 1×1 saat aplikasi belum punya teks)
// supaya tidak ada varian pipeline "dengan/tanpa tekstur".
@group(0) @binding(1) var atlas_mask: texture_2d<f32>;
@group(0) @binding(2) var atlas_color: texture_2d<f32>;
@group(0) @binding(3) var atlas_sampler: sampler;

struct Instance {
    // xy = pusat, zw = setengah ukuran (poin logis).
    @location(0) bounds: vec4<f32>,
    // Radius per sudut: kiri-atas, kanan-atas, kanan-bawah, kiri-bawah.
    // Sudah dikalikan faktor squircle dan dibatasi CPU-side.
    // GLYPH: slot yang sama membawa kotak UV [u0, v0, u1, v1].
    @location(1) radii: vec4<f32>,
    // Warna isi (shadow: warna bayangan; glyph: warna teks dari token theme),
    // straight alpha, ruang target.
    @location(2) background: vec4<f32>,
    // Warna border, straight alpha. Tidak dipakai instance glyph.
    @location(3) border: vec4<f32>,
    // x = tebal border (glyph: 0 = atlas mask, 1 = atlas warna),
    // y = eksponen superellipse, z = sigma blur,
    // w = jenis (0 = kotak, 1 = bayangan, 2 = glyph).
    @location(4) params: vec4<f32>,
};

struct Varying {
    @builtin(position) position: vec4<f32>,
    // Posisi fragmen relatif pusat kotak, poin logis.
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) half_size: vec2<f32>,
    @location(2) @interpolate(flat) radii: vec4<f32>,
    @location(3) @interpolate(flat) background: vec4<f32>,
    @location(4) @interpolate(flat) border: vec4<f32>,
    @location(5) @interpolate(flat) params: vec4<f32>,
};

// Empat titik triangle-strip: (-1,-1), (1,-1), (-1,1), (1,1).
fn corner_of(index: u32) -> vec2<f32> {
    return vec2<f32>(f32(index & 1u), f32((index >> 1u) & 1u)) * 2.0 - vec2<f32>(1.0);
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32, inst: Instance) -> Varying {
    let half_size = inst.bounds.zw;
    // Margin gambar: 3σ untuk ekor gaussian bayangan + 1 poin untuk anti-alias.
    // Glyph TIDAK diberi margin: kotaknya sudah persis sebesar bitmap di atlas
    // dan sudah disetel ke grid piksel fisik, jadi satu texel jatuh tepat pada
    // satu piksel layar — margin sekecil apa pun akan meregangkan UV dan
    // membuat teks lembek di layar 2×.
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

// Radius sudut yang berlaku untuk kuadran tempat fragmen ini berada.
// Sumbu y menghadap ke bawah, sesuai koordinat `rustui-paint`.
fn radius_for(local: vec2<f32>, radii: vec4<f32>) -> f32 {
    let kiri = local.x < 0.0;
    let atas = select(radii.y, radii.x, kiri);
    let bawah = select(radii.z, radii.w, kiri);
    return select(bawah, atas, local.y < 0.0);
}

// Norma-p dari vektor non-negatif: inilah satu-satunya tempat kedua mode
// geometri sudut berbeda. n = 2 memberi lingkaran (arc), n > 2 memberi
// superellipse (squircle).
fn norm_p(v: vec2<f32>, n: f32) -> f32 {
    if (n <= 2.0) {
        return length(v);
    }
    return pow(pow(v.x, n) + pow(v.y, n), 1.0 / n);
}

// Besar gradien norma-p. Untuk n > 2 bidangnya bukan jarak sejati (gradiennya
// mengecil ke arah diagonal), sehingga pita anti-alias akan melebar di sudut
// kalau tidak dinormalkan.
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

// Signed distance ke kotak bersudut membulat/squircle.
// Negatif di dalam, positif di luar, dalam poin logis.
fn sd_shape(local: vec2<f32>, half_size: vec2<f32>, radii: vec4<f32>, n: f32) -> f32 {
    let r = radius_for(local, radii);
    let q = abs(local) - half_size + vec2<f32>(r);
    let luar = max(q, vec2<f32>(0.0));
    let dalam = min(max(q.x, q.y), 0.0);
    return (dalam + norm_p(luar, n) - r) / norm_p_gradient(luar, n);
}

// Cakupan piksel dari jarak bertanda: pita selebar satu piksel layar.
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

// Aproksimasi erf (Abramowitz & Stegun 7.1.27), dua nilai sekaligus.
fn erf2(v: vec2<f32>) -> vec2<f32> {
    let s = sign(v);
    let a = abs(v);
    var r = 1.0 + (0.278393 + (0.230389 + 0.078108 * a * a) * a) * a;
    r = r * r;
    r = r * r;
    return s - s / r;
}

// Setengah lebar bentuk pada baris `y` (relatif pusat). Untuk sudut arc ini
// adalah sqrt(r² - dy²); untuk squircle dipakai norma-p yang sama dengan SDF,
// sehingga bayangan kartu squircle ikut squircle.
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

// Kotak membulat yang di-blur gaussian, cara Evan Wallace: integrasi analitik
// (erf) pada sumbu x, numerik pada sumbu y. Jauh lebih murah daripada blur
// dua-pass lewat texture, dan itulah yang membuat shadow ganda ala HIG
// gratis di 120 fps.
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
    // `fwidth` wajib dipanggil di aliran kontrol seragam — karena itu ia ada
    // di sini, sebelum percabangan jenis instance.
    let px = max(fwidth(sd), 1e-5);

    var warna: vec4<f32>;
    if (in.params.w > KIND_GLYPH) {
        // Kotak tujuan → kotak UV, keduanya axis-aligned: pemetaannya afin.
        // `textureSampleLevel` (bukan `textureSample`) supaya sampling tetap
        // sah di dalam percabangan — dan atlas memang tidak punya mip.
        let t = clamp(in.local / max(in.half_size, vec2<f32>(1e-6)) * 0.5 + vec2<f32>(0.5),
                      vec2<f32>(0.0), vec2<f32>(1.0));
        let uv = mix(in.radii.xy, in.radii.zw, t);
        if (in.params.x > ATLAS_COLOR) {
            // Emoji: warna datang dari atlas, alpha run tetap dihormati
            // supaya fade/disabled bekerja sama seperti pada teks biasa.
            let texel = textureSampleLevel(atlas_color, atlas_sampler, uv, 0.0);
            warna = premultiply(texel) * in.background.a;
        } else {
            // Teks biasa: atlas hanya menyimpan CAKUPAN, warnanya datang dari
            // token theme lewat instance — itulah sebabnya satu bitmap "a"
            // melayani label, secondary label, dan accent sekaligus.
            let cakupan = textureSampleLevel(atlas_mask, atlas_sampler, uv, 0.0).r;
            warna = premultiply(in.background) * cakupan;
        }
    } else if (in.params.w > KIND_SHADOW) {
        // Blur gaussian memakai satu radius; perbedaan antar sudut tidak
        // terlihat setelah di-blur, jadi dipakai rata-ratanya.
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
        // Border adalah cincin antara tepi luar dan tepi yang menyusut
        // setebal border — otomatis mengikuti squircle maupun arc.
        let dalam = coverage(sd + in.params.x, px);
        warna = premultiply(in.background) * dalam
            + premultiply(in.border) * max(luar - dalam, 0.0);
    }
    return warna;
}
