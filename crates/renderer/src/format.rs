//! Pemilihan konfigurasi surface — logika murni, bisa diuji tanpa GPU.
//!
//! Dipisah dari [`crate::surface`] justru supaya bisa di-unit-test: bagian
//! inilah yang menentukan apakah warna token tampil benar (sRGB) atau terlihat
//! "cuci" karena salah ruang warna.

use rustui_paint::Color;

/// Pilih format swapchain: utamakan format sRGB agar konversi gamma dikerjakan
/// hardware saat menulis, bukan ditebak-tebak di shader.
///
/// Di macOS/Metal ini akan jatuh ke `Bgra8UnormSrgb`.
pub(crate) fn choose_surface_format(
    available: &[wgpu::TextureFormat],
) -> Option<wgpu::TextureFormat> {
    available
        .iter()
        .copied()
        .find(|f| f.is_srgb())
        .or_else(|| available.first().copied())
}

/// Pilih mode komposit alpha: utamakan `Opaque` (window UI biasa), baru
/// `Inherit`, sisanya apa pun yang tersedia.
pub(crate) fn choose_alpha_mode(
    available: &[wgpu::CompositeAlphaMode],
) -> wgpu::CompositeAlphaMode {
    for pilihan in [
        wgpu::CompositeAlphaMode::Opaque,
        wgpu::CompositeAlphaMode::Inherit,
    ] {
        if available.contains(&pilihan) {
            return pilihan;
        }
    }
    available
        .first()
        .copied()
        .unwrap_or(wgpu::CompositeAlphaMode::Auto)
}

/// Terjemahkan warna token menjadi clear color GPU.
///
/// Ini titik konversi ruang warna yang MENGIKAT: token ditulis dalam sRGB
/// (§2.7), sedangkan attachment berformat `*Srgb` mengharapkan nilai **linear**
/// karena hardware yang melakukan encoding balik. Melewatkan konversi ini
/// membuat setiap latar tampak jauh lebih terang dari tokennya.
pub(crate) fn clear_color(color: Color, format: wgpu::TextureFormat) -> wgpu::Color {
    let [r, g, b, a] = if format.is_srgb() {
        color.to_linear()
    } else {
        color.components()
    };
    wgpu::Color {
        r: r as f64,
        g: g as f64,
        b: b as f64,
        a: a as f64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::TextureFormat as Tf;

    #[test]
    fn srgb_diutamakan_walau_bukan_yang_pertama() {
        let tersedia = [Tf::Bgra8Unorm, Tf::Bgra8UnormSrgb];
        assert_eq!(choose_surface_format(&tersedia), Some(Tf::Bgra8UnormSrgb));
    }

    #[test]
    fn tanpa_srgb_ambil_yang_pertama() {
        let tersedia = [Tf::Rgba16Float, Tf::Bgra8Unorm];
        assert_eq!(choose_surface_format(&tersedia), Some(Tf::Rgba16Float));
    }

    #[test]
    fn tanpa_format_sama_sekali_none() {
        assert_eq!(choose_surface_format(&[]), None);
    }

    #[test]
    fn alpha_opaque_diutamakan() {
        let tersedia = [
            wgpu::CompositeAlphaMode::PostMultiplied,
            wgpu::CompositeAlphaMode::Opaque,
        ];
        assert_eq!(
            choose_alpha_mode(&tersedia),
            wgpu::CompositeAlphaMode::Opaque
        );
    }

    #[test]
    fn alpha_jatuh_ke_inherit_lalu_apa_saja() {
        assert_eq!(
            choose_alpha_mode(&[
                wgpu::CompositeAlphaMode::PostMultiplied,
                wgpu::CompositeAlphaMode::Inherit
            ]),
            wgpu::CompositeAlphaMode::Inherit
        );
        assert_eq!(
            choose_alpha_mode(&[wgpu::CompositeAlphaMode::PreMultiplied]),
            wgpu::CompositeAlphaMode::PreMultiplied
        );
        assert_eq!(choose_alpha_mode(&[]), wgpu::CompositeAlphaMode::Auto);
    }

    #[test]
    fn target_srgb_menerima_nilai_linear() {
        let c = clear_color(Color::srgb(0.5, 0.5, 0.5), Tf::Bgra8UnormSrgb);
        assert!((c.r - 0.214_041).abs() < 1e-4, "r = {}", c.r);
    }

    #[test]
    fn target_non_srgb_menerima_nilai_apa_adanya() {
        let c = clear_color(Color::srgb(0.5, 0.5, 0.5), Tf::Bgra8Unorm);
        assert!((c.r - 0.5).abs() < 1e-6);
    }

    #[test]
    fn hitam_dan_putih_tetap_hitam_dan_putih() {
        for format in [Tf::Bgra8UnormSrgb, Tf::Bgra8Unorm] {
            let hitam = clear_color(Color::BLACK, format);
            let putih = clear_color(Color::WHITE, format);
            assert!(hitam.r.abs() < 1e-6);
            assert!((putih.r - 1.0).abs() < 1e-6);
            assert!((putih.a - 1.0).abs() < 1e-6);
        }
    }
}
