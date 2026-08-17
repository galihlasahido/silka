//! Demo page: the **Tier 0/1 layout primitives and media** — `spacer`,
//! `divider`, `stack`, `align`/`center`, `aspect_ratio`, `image` and `icon`
//! (`KOMPONEN.md` Tier 0 and Tier 1).
//!
//! These are the smallest components in the catalogue and the last ones to be
//! written, which is why this page exists at all: until it did, the gallery and
//! the dashboard were both assembling separators out of constrained empty boxes
//! and gaps out of `expanded(fixed(0.0, 0.0))`. Every one of those in both
//! example applications is gone; this page is what replaced them, shown as the
//! components they always should have been.
//!
//! | What it proves | How to check it |
//! |---|---|
//! | A divider is a token, not a colour | Switch preset or appearance in the top bar: the hairline follows |
//! | A divider is a **separator** to a screen reader | The a11y dump shows `separator`, which no hand-rolled version ever did |
//! | An inset mirrors | The inset divider keeps its blank end on the reading start in an RTL document |
//! | A stack is the z-axis | The badge sits **on** the tile, not beside it — and the alignment is what decides which corner |
//! | `aspect_ratio` ties one axis to the other | Resize the window: the 16:9 frame stays 16:9 while its width changes |
//! | An icon is coverage, not colour | The same chevron appears in three colours and is **one** bitmap in the atlas |
//! | Cropping is free | The `cover` specimen is cropped through the source rect; no pixel is resampled on the CPU |

use std::cell::RefCell;

use silka_core::app::{BuildCtx, ScaleFactor};
use silka_core::signals::Signal;
use silka_core::tree::{BoxConstraints, CrossAlign};
use silka_core::view::{constrained, div, fixed, row, View};
use silka_paint::ImageId;
use silka_theme::{ColorToken, FontToken, RadiusToken, SpaceToken, Theme};
use silka_widgets::{
    active_images, align, aspect_ratio, center, divider_in, icon_in, image_in, spacer, stack,
    text_in, Alignment, Fonts, IconName, Images, ASPECT_16_9, ASPECT_4_3, ASPECT_SQUARE,
};

/// The page title.
pub const JUDUL: &str = "Tata letak & media";

/// The a11y name of the divider that opens the separator section — proof that
/// a divider can carry a name at all.
pub const NAMA_PEMISAH: &str = "Batas bagian";

/// The a11y name of the specimen picture.
pub const NAMA_GAMBAR: &str = "Spesimen papan catur";

/// Width of the page content, in spacing steps.
const LEBAR_LANGKAH: f32 = 120.0;

/// The specimen bitmap's size, in pixels.
const SPESIMEN_W: u32 = 64;
/// The specimen bitmap's height, in pixels.
const SPESIMEN_H: u32 = 32;

thread_local! {
    /// The specimen bitmap, inserted **once** for the life of the thread.
    ///
    /// A page function runs on every rebuild, and inserting a bitmap on every
    /// rebuild would grow the atlas without limit — the exact mistake the atlas
    /// cannot protect an application from, so the application must not make it.
    static SPESIMEN: RefCell<Option<ImageId>> = const { RefCell::new(None) };
}

/// A 2:1 coverage mask: a solid frame around a checkerboard.
///
/// A **mask** rather than a colour bitmap, on purpose: it goes into the atlas
/// as coverage, and the theme token tints it at draw time — so even the demo
/// picture on this page holds no colour of its own (§2.6). The shape is chosen
/// so that a crop is unmistakable: the frame disappears on the cropped edges.
fn spesimen_alpha() -> Vec<u8> {
    let mut alpha = vec![0u8; (SPESIMEN_W * SPESIMEN_H) as usize];
    for y in 0..SPESIMEN_H {
        for x in 0..SPESIMEN_W {
            let border = x < 2 || y < 2 || x >= SPESIMEN_W - 2 || y >= SPESIMEN_H - 2;
            let papan = ((x / 8) + (y / 8)) % 2 == 0;
            alpha[(y * SPESIMEN_W + x) as usize] = if border {
                255
            } else if papan {
                190
            } else {
                70
            };
        }
    }
    alpha
}

/// The specimen's handle, inserting it on first use.
fn spesimen(images: &Images) -> Option<ImageId> {
    SPESIMEN.with(|slot| {
        if let Some(id) = *slot.borrow() {
            return Some(id);
        }
        let id = images.insert_mask(SPESIMEN_W, SPESIMEN_H, &spesimen_alpha())?;
        *slot.borrow_mut() = Some(id);
        Some(id)
    })
}

/// The view tree for the whole page.
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());
    let images = active_images();
    images.set_scale_factor(dpi.get());

    div()
        .items_start()
        .gap_6()
        .p_8()
        .child(
            text_in(fonts, JUDUL)
                .font(FontToken::Title2)
                .font_semibold()
                .text_color(ColorToken::Label)
                .single_line(),
        )
        .child(
            text_in(
                fonts,
                "Tujuh komponen paling dasar di katalog, dan yang paling akhir \
                 ditulis. Sebelum ada, galeri dan dashboard sama-sama merakit \
                 garis pemisah dari kotak kosong ber-constraint dan celah dari \
                 flex child berukuran nol. Semuanya sudah diganti — halaman ini \
                 penggantinya.",
            )
            .text_base()
            .text_color(ColorToken::SecondaryLabel)
            .max_width(t.space(LEBAR_LANGKAH)),
        )
        .child(judul_bagian(fonts, "Spacer & divider"))
        .child(bagian_pemisah(fonts, &t))
        .child(judul_bagian(fonts, "Stack (sumbu z)"))
        .child(bagian_stack(fonts, &t))
        .child(judul_bagian(fonts, "Align & center"))
        .child(bagian_align(fonts, &t))
        .child(judul_bagian(fonts, "Aspect ratio"))
        .child(bagian_rasio(fonts, &t))
        .child(judul_bagian(fonts, "Icon"))
        .child(bagian_ikon(fonts, &t, &images))
        .child(judul_bagian(fonts, "Image — fit mode & rounded clip"))
        .child(bagian_gambar(fonts, &t, &images))
        .into()
}

/// A section heading.
fn judul_bagian(fonts: &Fonts, judul: &str) -> View {
    text_in(fonts, judul)
        .font(FontToken::Caption1)
        .font_semibold()
        .text_color(ColorToken::TertiaryLabel)
        .single_line()
        .into()
}

/// A small caption under a specimen.
fn keterangan(fonts: &Fonts, teks: &str) -> View {
    text_in(fonts, teks)
        .text_xs()
        .text_color(ColorToken::TertiaryLabel)
        .single_line()
        .into()
}

/// A specimen with its caption underneath.
fn spesimen_kolom(fonts: &Fonts, isi: View, caption: &str) -> View {
    div()
        .items_start()
        .gap_1()
        .child(isi)
        .child(keterangan(fonts, caption))
        .into()
}

// ---------------------------------------------------------------------------
// Spacer & divider
// ---------------------------------------------------------------------------

/// A card whose header is pushed apart by a spacer and separated by a divider.
fn bagian_pemisah(fonts: &Fonts, t: &Theme) -> View {
    let judul_baris = |kiri: &str, kanan: &str| -> View {
        row([
            text_in(fonts, kiri)
                .text_base()
                .text_color(ColorToken::Label)
                .single_line()
                .into(),
            // The whole reason `spacer()` exists: the gap belongs to the layout
            // engine, and there is no number here to get wrong.
            View::from(spacer()),
            text_in(fonts, kanan)
                .text_sm()
                .text_color(ColorToken::SecondaryLabel)
                .single_line()
                .into(),
        ])
        .cross(CrossAlign::Center)
        .px_4()
        .py_3()
        .into()
    };

    let kartu = div()
        .items_stretch()
        .bg(ColorToken::Surface)
        .rounded_lg()
        .border_1()
        .border_color(ColorToken::Separator)
        .child(judul_baris("Pinjaman aktif", "128"))
        // A named divider: the one case where a separator earns a name, because
        // it genuinely opens a section.
        .child(divider_in(t).label(NAMA_PEMISAH))
        .child(judul_baris("Menunggu akad", "12"))
        // …and an inset one, the shape a list separator takes when it lines up
        // with the row's text. The inset is reading-relative, so it mirrors.
        .child(divider_in(t).inset_start(SpaceToken::S4))
        .child(judul_baris("Ditolak", "3"))
        .child(divider_in(t).inset(SpaceToken::S4))
        .child(judul_baris("Lunas", "1.204"));

    let vertikal = row([
        View::from(keterangan(fonts, "kiri")),
        divider_in(t).vertical().into(),
        View::from(keterangan(fonts, "kanan")),
    ])
    .gap_3()
    .cross(CrossAlign::Stretch);

    div()
        .items_start()
        .gap_4()
        .child(constrained(
            BoxConstraints::new(
                t.space(LEBAR_LANGKAH * 0.5),
                t.space(LEBAR_LANGKAH * 0.5),
                0.0,
                f32::INFINITY,
            ),
            kartu,
        ))
        .child(spesimen_kolom(
            fonts,
            constrained(
                BoxConstraints::new(0.0, f32::INFINITY, t.space(8.0), t.space(8.0)),
                vertikal,
            )
            .into(),
            "divider().vertical()",
        ))
        .into()
}

// ---------------------------------------------------------------------------
// Stack
// ---------------------------------------------------------------------------

/// A tile with a count badge pinned to one corner — the canonical stack.
fn bagian_stack(fonts: &Fonts, t: &Theme) -> View {
    let ubin = |alignment: Alignment, caption: &str| -> View {
        let dasar = div()
            .items_center()
            .justify_center()
            .bg(ColorToken::Surface)
            .rounded_lg()
            .border_1()
            .border_color(ColorToken::Separator)
            .child(
                text_in(fonts, "Inbox")
                    .text_sm()
                    .text_color(ColorToken::SecondaryLabel)
                    .single_line(),
            );

        let lencana = div()
            .items_center()
            .justify_center()
            .px_2()
            .py_px()
            .bg(ColorToken::Accent)
            .rounded_full()
            .child(
                text_in(fonts, "9+")
                    .text_xs()
                    .font_semibold()
                    .text_color(ColorToken::OnAccent)
                    .single_line(),
            );

        spesimen_kolom(
            fonts,
            constrained(
                BoxConstraints::new(t.space(22.0), t.space(22.0), t.space(14.0), t.space(14.0)),
                // `expand()` hands the whole box to **both** children: the tile
                // fills it, and the badge's own `align` is what puts the badge in
                // a corner. That is the pattern `stack`'s docs describe for a
                // child that needs a different corner from its siblings.
                stack([
                    View::from(dasar),
                    View::from(align(lencana).alignment(alignment)),
                ])
                .expand(),
            )
            .into(),
            caption,
        )
    };

    row([
        ubin(Alignment::TOP_END, "TOP_END"),
        ubin(Alignment::TOP_START, "TOP_START"),
        ubin(Alignment::BOTTOM_END, "BOTTOM_END"),
        ubin(Alignment::CENTER, "CENTER"),
    ])
    .gap_4()
    .cross(CrossAlign::Start)
    .into()
}

// ---------------------------------------------------------------------------
// Align & center
// ---------------------------------------------------------------------------

/// One box positioned inside a bigger one, nine ways.
fn bagian_align(fonts: &Fonts, t: &Theme) -> View {
    let sel = |alignment: Alignment, caption: &str| -> View {
        let titik = fixed(t.space(3.0), t.space(3.0))
            .bg(ColorToken::Accent)
            .rounded_sm();
        spesimen_kolom(
            fonts,
            constrained(
                BoxConstraints::new(t.space(16.0), t.space(16.0), t.space(11.0), t.space(11.0)),
                align(titik)
                    .alignment(alignment)
                    .bg(ColorToken::SurfaceSunken)
                    .rounded_md(),
            )
            .into(),
            caption,
        )
    };

    // `center(x)` is `align(x)` at its default — the name is the documentation.
    let tengah = spesimen_kolom(
        fonts,
        constrained(
            BoxConstraints::new(t.space(16.0), t.space(16.0), t.space(11.0), t.space(11.0)),
            center(
                text_in(fonts, "center()")
                    .text_xs()
                    .text_color(ColorToken::SecondaryLabel)
                    .single_line(),
            )
            .bg(ColorToken::SurfaceSunken)
            .rounded_md(),
        )
        .into(),
        "center()",
    );

    row([
        sel(Alignment::TOP_START, "TOP_START"),
        sel(Alignment::TOP_CENTER, "TOP_CENTER"),
        sel(Alignment::CENTER_END, "CENTER_END"),
        sel(Alignment::BOTTOM_CENTER, "BOTTOM_CENTER"),
        tengah,
    ])
    .gap_4()
    .cross(CrossAlign::Start)
    .into()
}

// ---------------------------------------------------------------------------
// Aspect ratio
// ---------------------------------------------------------------------------

/// Three frames of the same width and three different shapes.
fn bagian_rasio(fonts: &Fonts, t: &Theme) -> View {
    let bingkai = |ratio: f32, caption: &str| -> View {
        spesimen_kolom(
            fonts,
            constrained(
                BoxConstraints::new(t.space(24.0), t.space(24.0), 0.0, f32::INFINITY),
                aspect_ratio(
                    ratio,
                    div()
                        .bg(ColorToken::SurfaceSunken)
                        .rounded_md()
                        .border_1()
                        .border_color(ColorToken::Separator),
                ),
            )
            .into(),
            caption,
        )
    };

    row([
        bingkai(ASPECT_16_9, "16:9"),
        bingkai(ASPECT_4_3, "4:3"),
        bingkai(ASPECT_SQUARE, "1:1"),
    ])
    .gap_4()
    .cross(CrossAlign::Start)
    .into()
}

// ---------------------------------------------------------------------------
// Icon
// ---------------------------------------------------------------------------

/// The whole built-in set, plus the proof that one bitmap serves every colour.
fn bagian_ikon(fonts: &Fonts, t: &Theme, images: &Images) -> View {
    let semua = div()
        .flex()
        .wrap()
        .items_start()
        .gap_4()
        .children(IconName::ALL.map(|name| {
            View::from(
                div()
                    .items_center()
                    .gap_1()
                    .child(icon_in(images, t, name).md().color(ColorToken::Label))
                    .child(keterangan(fonts, name.name())),
            )
        }));

    // Three colours, one bitmap: the icon is coverage and the token colours it,
    // exactly as one glyph bitmap serves every text colour.
    let warna = row([
        ColorToken::Label,
        ColorToken::SecondaryLabel,
        ColorToken::Accent,
        ColorToken::Destructive,
    ]
    .map(|token| View::from(icon_in(images, t, IconName::ChevronRight).lg().color(token))))
    .gap_2()
    .cross(CrossAlign::Center);

    div()
        .items_start()
        .gap_4()
        .child(semua)
        .child(spesimen_kolom(
            fonts,
            warna.into(),
            "satu bitmap, empat token warna",
        ))
        .into()
}

// ---------------------------------------------------------------------------
// Image
// ---------------------------------------------------------------------------

/// The same 2:1 specimen in four fits, inside one square box.
fn bagian_gambar(fonts: &Fonts, t: &Theme, images: &Images) -> View {
    let Some(id) = spesimen(images) else {
        return keterangan(fonts, "atlas gambar penuh — spesimen tidak masuk");
    };

    let sisi = t.space(18.0);
    let kotak = |isi: View, caption: &str| -> View {
        spesimen_kolom(
            fonts,
            constrained(
                BoxConstraints::new(sisi, sisi, sisi, sisi),
                // A stack in `expand()` mode is the honest square frame here: the
                // picture is handed exactly the box, so the fit mode is the only
                // thing deciding what happens to it.
                stack([isi])
                    .expand()
                    .bg(ColorToken::SurfaceSunken)
                    .rounded_md(),
            )
            .into(),
            caption,
        )
    };

    row([
        kotak(
            image_in(images, id)
                .theme(t)
                .contain()
                .expand()
                .tint(ColorToken::Accent)
                .label(NAMA_GAMBAR)
                .into(),
            "contain",
        ),
        kotak(
            image_in(images, id)
                .theme(t)
                .cover()
                .expand()
                .tint(ColorToken::Accent)
                .into(),
            "cover",
        ),
        kotak(
            image_in(images, id)
                .theme(t)
                .fill()
                .expand()
                .tint(ColorToken::Accent)
                .into(),
            "fill",
        ),
        kotak(
            image_in(images, id)
                .theme(t)
                .cover()
                .expand()
                .rounded(RadiusToken::Xl)
                .tint(ColorToken::Accent)
                .into(),
            "cover + rounded_xl",
        ),
    ])
    .gap_4()
    .cross(CrossAlign::Start)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::AccessRole;
    use silka_core::app::AppRuntime;
    use silka_paint::{Command, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};

    const VIEWPORT: Size = Size::new(1100.0, 1400.0);

    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    fn ui(theme: Theme, fonts: &Fonts) -> AppRuntime {
        let untuk_view = fonts.clone();
        let mut ui = headless_app(theme, move |cx| halaman(cx, &untuk_view))
            .sized(VIEWPORT.width, VIEWPORT.height);
        ui.frame();
        ui
    }

    #[test]
    fn setiap_ikon_bawaan_punya_bitmap() {
        let images = Images::new();
        for name in IconName::ALL {
            assert!(
                images.icon(name.name(), name.path(), 24.0, 20).is_some(),
                "ikon '{}' tidak bisa dirasterisasi",
                name.name()
            );
        }
    }

    #[test]
    fn halaman_menggambar_teks_kotak_dan_gambar() {
        let f = fonts();
        let ui = ui(Theme::cupertino(Appearance::Dark), &f);
        let perintah = ui.scene().commands();
        assert!(
            perintah.iter().any(|c| matches!(c, Command::GlyphRun(_))),
            "tidak ada satu pun glyph tergambar"
        );
        assert!(
            perintah.iter().any(|c| matches!(c, Command::Quad(_))),
            "tidak ada satu pun kotak tergambar"
        );
        assert!(
            perintah.iter().any(|c| matches!(c, Command::Image(_))),
            "ikon dan gambar tidak menghasilkan satu pun perintah tekstur"
        );
    }

    #[test]
    fn divider_terbaca_sebagai_separator_oleh_pembaca_layar() {
        let f = fonts();
        let ui = ui(Theme::cupertino(Appearance::Light), &f);
        let pohon = ui.access_tree();
        let e = pohon
            .find_label(NAMA_PEMISAH)
            .unwrap_or_else(|| panic!("{}", pohon.dump()));
        assert_eq!(
            e.node.role,
            AccessRole::Separator,
            "peran Separator sudah lama ada di kosakata dan halaman ini yang \
             pertama memakainya"
        );
    }

    #[test]
    fn gambar_bernama_adalah_konten_dan_ikon_tanpa_nama_adalah_dekorasi() {
        let f = fonts();
        let ui = ui(Theme::cupertino(Appearance::Dark), &f);
        let pohon = ui.access_tree();
        let gambar = pohon
            .find_label(NAMA_GAMBAR)
            .unwrap_or_else(|| panic!("{}", pohon.dump()));
        assert_eq!(gambar.node.role, AccessRole::Image);

        // Every icon on this page is decorative — the caption beside it already
        // says the same thing — so exactly one node has the Image role.
        let jumlah = pohon
            .entries()
            .iter()
            .filter(|e| e.node.role == AccessRole::Image)
            .count();
        assert_eq!(
            jumlah,
            1,
            "ikon dekoratif tidak boleh diumumkan:\n{}",
            pohon.dump()
        );
    }

    #[test]
    fn warna_pemisah_ikut_token_di_kedua_preset() {
        // The claim the page makes at the top: not one colour lives in this
        // file, so a preset switch moves the hairline with it.
        for preset in [Preset::Cupertino, Preset::Tailwind] {
            let terang = Theme::new(preset, Appearance::Light);
            let gelap = Theme::new(preset, Appearance::Dark);
            assert_ne!(
                terang.color_of(ColorToken::Separator),
                gelap.color_of(ColorToken::Separator),
                "{preset:?}"
            );
        }
    }

    #[test]
    fn spesimen_hanya_masuk_atlas_sekali() {
        // A page function runs on every rebuild; inserting the bitmap each time
        // would grow the atlas without limit.
        let images = Images::new();
        SPESIMEN.with(|s| *s.borrow_mut() = None);
        let a = spesimen(&images).expect("masuk");
        let b = spesimen(&images).expect("masuk");
        assert_eq!(a, b);
        SPESIMEN.with(|s| *s.borrow_mut() = None);
    }
}
