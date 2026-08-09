//! Halaman demo: **grid kartu yang digambar lewat siklus hidup reaktif**.
//!
//! Isi visualnya sengaja sama dengan halaman [`crate::kartu`] — squircle di
//! kiri, arc di kanan, radius dan elevasi naik tiap baris — tapi jalannya
//! berbeda dan itulah gunanya halaman ini: di sini **tidak ada satu pun
//! `Scene` yang disusun tangan**. Yang ditulis hanyalah pohon view; scene-nya
//! lahir dari `signals → view-diff → layout → paint` di dalam
//! [`rustui_core::app::AppRuntime`] (REKOMENDASI §2, §3.5).
//!
//! Dua hal yang ikut terbukti dengan menjalankan halaman ini:
//!
//! 1. **Layout dihitung mesin**, bukan aritmetika di kode halaman. Tidak ada
//!    `padding * 2.0 - gap` di berkas ini; posisinya datang dari `column`/`row`
//!    dan `expanded()` (§3.4).
//! 2. **Theme adalah signal.** Dark mode OS yang berubah menulis
//!    `Signal<Theme>`, dan yang dibangun ulang hanyalah komponen yang
//!    benar-benar membacanya (§2.7).

use rustui_core::app::{component, BuildCtx};
use rustui_core::signals::{Key, Signal};
use rustui_core::tree::CrossAlign;
use rustui_core::view::{column, expanded, fixed, row, View};
use rustui_paint::{CornerStyle, Corners, Insets, ShadowPair};
use rustui_theme::Theme;

/// Berapa baris kartu (satu baris = satu radius + satu elevasi).
pub const BARIS: usize = 4;

/// Pohon view seluruh halaman — inilah yang diserahkan ke `run_app`.
///
/// Dibaca di scope akar: theme yang berganti membangun ulang halaman ini
/// seluruhnya, dan itu memang yang diinginkan karena setiap warnanya token.
pub fn halaman(cx: &BuildCtx) -> View {
    let theme: Signal<Theme> = cx.expect_env();
    let t = theme.get();
    let gap = t.space(4.0);

    column(
        (0..BARIS)
            .map(|baris| {
                View::from(expanded(
                    row([expanded(kartu(baris, 0)), expanded(kartu(baris, 1))])
                        .spacing(gap)
                        // Kartu setinggi barisnya, bukan setinggi isinya.
                        .cross(CrossAlign::Stretch),
                ))
            })
            .collect::<Vec<View>>(),
    )
    .spacing(gap)
    // Tiap baris selebar halaman; tanpa ini `expanded()` di dalamnya tidak
    // punya ruang untuk dibagi.
    .cross(CrossAlign::Stretch)
    .padding(Insets::all(t.space(6.0)))
    .into()
}

/// Satu kartu sebagai komponen tersendiri.
///
/// Tiap kartu punya scope-nya sendiri, jadi kelak ketika satu kartu punya state
/// (hover, pressed) hanya kartu itu yang dibangun ulang — bukan seluruh grid.
fn kartu(baris: usize, kolom: usize) -> View {
    component(Key::num((baris * 2 + kolom) as i64), move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let (radius, shadow) = gaya_baris(&t, baris);
        // Kolom kiri squircle, kolom kanan arc — halaman ini satu-satunya yang
        // boleh memilih bentuk sudut sendiri, karena tugasnya membandingkan.
        let style = if kolom == 0 {
            CornerStyle::squircle()
        } else {
            CornerStyle::Arc
        };
        // Ukuran nol: `expanded()` di atasnya memberi constraints tight, jadi
        // kartu mengisi selnya. Angka tata letaknya milik mesin layout.
        fixed(0.0, 0.0)
            .background(t.color.surface)
            .corners(Corners::uniform(radius, style))
            .border(t.space(0.25), t.color.separator)
            .shadow(shadow)
            .into()
    })
}

/// Radius + elevasi untuk sebuah baris — keduanya token, bukan angka lepas.
fn gaya_baris(t: &Theme, baris: usize) -> (f32, ShadowPair) {
    match baris {
        0 => (t.radius.sm, t.shadow.sm),
        1 => (t.radius.md, t.shadow.sm),
        2 => (t.radius.lg, t.shadow.md),
        _ => (t.radius.xl, t.shadow.lg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustui_core::app::{app, AppRuntime};
    use rustui_paint::{Command, Quad, Size};
    use rustui_theme::{Appearance, Preset};

    const VIEWPORT: Size = Size::new(1024.0, 720.0);

    /// Aplikasi headless dengan titipan theme yang sama seperti `run_app`.
    fn ui(theme: Theme) -> AppRuntime {
        app(halaman)
            .with_env(move |rt| rt.signal(theme))
            .clear_color(theme.color.background)
            .sized(VIEWPORT.width, VIEWPORT.height)
    }

    fn kotak(ui: &AppRuntime) -> Vec<Quad> {
        ui.scene()
            .commands()
            .iter()
            .filter_map(|c| match c {
                Command::Quad(q) => Some(q.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn satu_kartu_per_sel_dan_semuanya_bertumpu_pada_bayangan_ganda() {
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
        ui.frame();
        assert_eq!(kotak(&ui).len(), BARIS * 2);
        // Dua bayangan + satu kotak per kartu, sama seperti halaman `kartu`.
        assert_eq!(ui.scene().len(), BARIS * 2 * 3);
    }

    #[test]
    fn tata_letak_dihitung_mesin_bukan_oleh_halaman() {
        let mut ui = ui(Theme::cupertino(Appearance::Light));
        ui.frame();
        let k = kotak(&ui);
        for baris in k.chunks(2) {
            // Kiri dan kanan sama lebar, sejajar, dan tidak saling menimpa.
            assert_eq!(baris[0].rect.size, baris[1].rect.size);
            assert_eq!(baris[0].rect.min_y(), baris[1].rect.min_y());
            assert!(baris[0].rect.max_x() <= baris[1].rect.min_x() + 1e-3);
        }
        for dua in k.chunks(2).collect::<Vec<_>>().windows(2) {
            assert!(dua[0][0].rect.max_y() <= dua[1][0].rect.min_y() + 1e-3);
        }
        // Semuanya di dalam viewport, dan tidak ada yang berukuran nol.
        for q in &k {
            assert!(q.rect.min_x() >= 0.0 && q.rect.min_y() >= 0.0, "{q:?}");
            assert!(q.rect.max_x() <= VIEWPORT.width + 1e-3, "{q:?}");
            assert!(q.rect.max_y() <= VIEWPORT.height + 1e-3, "{q:?}");
            assert!(!q.rect.size.is_empty(), "{q:?}");
        }
    }

    #[test]
    fn kolom_kiri_squircle_kolom_kanan_arc() {
        let mut ui = ui(Theme::tailwind(Appearance::Light));
        ui.frame();
        for baris in kotak(&ui).chunks(2) {
            assert_eq!(baris[0].corners.style, CornerStyle::squircle());
            assert_eq!(baris[1].corners.style, CornerStyle::Arc);
            assert_eq!(baris[0].corners.radii, baris[1].corners.radii);
        }
    }

    #[test]
    fn warna_selalu_datang_dari_token() {
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let mut ui = ui(t);
                ui.frame();
                assert_eq!(ui.scene().clear_color(), t.color.background);
                for q in kotak(&ui) {
                    assert_eq!(q.background, t.color.surface, "{preset:?} {appearance:?}");
                    assert_eq!(q.border_color, t.color.separator);
                    assert_eq!(q.border_width, t.space(0.25));
                }
            }
        }
    }

    #[test]
    fn ganti_theme_membangun_ulang_halaman_dan_idle_lagi_sesudahnya() {
        let terang = Theme::cupertino(Appearance::Light);
        let mut ui = ui(terang);
        ui.frame();
        assert!(ui.is_idle(), "halaman statis tidak menyisakan pekerjaan");

        let gelap = Theme::cupertino(Appearance::Dark);
        let signal: Signal<Theme> = ui.env().expect("theme dititipkan di Env");
        signal.set(gelap);
        assert!(!ui.is_idle(), "theme berubah menjadwalkan tepat satu frame");

        ui.set_clear_color(gelap.color.background);
        let laporan = ui.frame();
        assert_eq!(laporan.rebuilt, 1, "akar yang membaca theme");
        assert_eq!(laporan.diff.created, 0, "tidak ada node yang lahir ulang");
        assert_eq!(laporan.diff.removed, 0);
        for q in kotak(&ui) {
            assert_eq!(q.background, gelap.color.surface);
        }
        assert!(ui.is_idle());
    }
}
