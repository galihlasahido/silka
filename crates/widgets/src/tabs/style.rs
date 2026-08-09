//! Resolusi token → nilai konkret untuk `tabs` (§2.6, §2.7).
//!
//! Ini satu-satunya berkas komponen ini yang boleh menyebut [`Theme`]. Node
//! render di [`super::list`] dan [`super::item`] hanya menerima
//! [`TabsStyle`] yang **sudah** berisi warna, radius, dan jarak yang jadi —
//! aturan yang sama seperti [`silka_core::tree::Decoration`]: mesin tidak
//! punya pendapat tentang warna, jadi preset Cupertino/Tailwind berganti tanpa
//! satu baris pun berubah di kode node.
//!
//! Geometri sudut ikut sebagai **parameter** (squircle di Cupertino, arc di
//! Tailwind) — bukan konstanta, karena bentuk itu mengalir sampai ke shader
//! *dan* ke hit-testing (§2.7, §3.6).

use silka_core::tree::{Decoration, FocusRing};
use silka_paint::{Color, CornerRadii, Corners, Insets, Rect, ShadowPair};
use silka_theme::Theme;

use crate::button::MIN_HIT_TARGET;

/// Varian visual deretan tab (`KOMPONEN.md` Tier 3: segmented/underline/
/// enclosed).
///
/// Ketiganya berbagi **satu** mesin: yang berbeda hanya nilai token yang
/// diresolusi di sini dan bentuk kotak indikatornya
/// ([`TabsStyle::indicator_rect`]). Tidak ada satu pun dari mereka yang punya
/// jalur layout atau input sendiri.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TabsVariant {
    /// Thumb geser di dalam sebuah "sumur" — `NSSegmentedControl`/
    /// `UISegmentedControl`. Segmen berlebar sama.
    #[default]
    Segmented,
    /// Garis tebal di bawah tab aktif — gaya shadcn/ui dan toolbar web.
    Underline,
    /// Tab berbentuk map/folder yang menyatu dengan panel di bawahnya.
    Enclosed,
}

impl TabsVariant {
    /// Ketiga varian — dipakai gallery dan uji lintas-varian.
    pub const ALL: [TabsVariant; 3] = [
        TabsVariant::Segmented,
        TabsVariant::Underline,
        TabsVariant::Enclosed,
    ];

    /// Nama pendek untuk CLI/gallery/debug.
    pub const fn name(self) -> &'static str {
        match self {
            TabsVariant::Segmented => "segmented",
            TabsVariant::Underline => "underline",
            TabsVariant::Enclosed => "enclosed",
        }
    }
}

/// Seluruh nilai visual `tabs`, **sudah diresolusi** dari token theme.
///
/// Dipisah dari builder supaya bisa diuji tanpa render tree sama sekali:
/// pertanyaan "apakah indikator underline benar-benar menempel di tepi bawah
/// tab yang dipilih" tidak butuh GPU, tidak butuh window, dan tidak butuh
/// pohon — hanya [`TabsStyle::indicator_rect`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabsStyle {
    /// Varian yang menentukan bentuk indikator.
    pub variant: TabsVariant,

    /// Latar seluruh deretan (sumur segmented; kosong untuk varian lain).
    pub track: Decoration,
    /// Garis rambut selebar deretan di tepi bawah (underline & enclosed).
    pub rail: Option<Color>,
    /// Tebal garis rambut itu, poin logis.
    pub rail_thickness: f32,

    /// Latar, border, sudut, dan bayangan indikator yang bergerak.
    pub indicator: Decoration,
    /// Jarak indikator dari tepi kotak tab yang sedang dipilih.
    pub indicator_inset: Insets,
    /// Tebal indikator untuk varian [`TabsVariant::Underline`].
    pub indicator_thickness: f32,

    /// Cincin fokus keyboard (token `focus_ring`).
    pub focus_ring: FocusRing,

    /// Jarak di dalam tepi deretan.
    pub padding: Insets,
    /// Jarak antar tab.
    pub spacing: f32,
    /// Tinggi minimum satu tab — hit target HIG (`KOMPONEN.md` DoD).
    pub min_height: f32,
    /// Semua tab selebar yang terlebar (rasa `NSSegmentedControl`).
    pub equal_widths: bool,

    /// Bentuk sudut satu tab: dipakai sorotan hover **dan** hit-testing (§3.6).
    pub tab_corners: Corners,
    /// Jarak di dalam tepi satu tab.
    pub tab_padding: Insets,

    /// Sorotan saat penunjuk di atas sebuah tab.
    pub hover: Color,
    /// Sorotan saat tab ditekan.
    pub pressed: Color,

    /// Warna label tab yang tidak dipilih.
    pub label: Color,
    /// Warna label tab yang sedang dipilih.
    pub selected_label: Color,
    /// Warna label tab yang dimatikan.
    pub disabled_label: Color,
    /// Ukuran font label, poin logis.
    pub label_size: f32,
}

impl TabsStyle {
    /// Resolusi seluruh token untuk sebuah varian.
    ///
    /// Tidak ada satu angka warna pun yang lahir di sini: semuanya turunan
    /// [`Theme`], sehingga kedua preset otomatis benar dan dark mode ikut
    /// tanpa cabang `if`.
    pub fn from_theme(theme: &Theme, variant: TabsVariant) -> Self {
        let rambut = theme.space(0.25);
        let dasar = Self {
            variant,
            track: Decoration::NONE,
            rail: None,
            rail_thickness: rambut,
            indicator: Decoration::NONE,
            indicator_inset: Insets::ZERO,
            indicator_thickness: theme.space(0.5),
            focus_ring: FocusRing::new(theme.space(0.5), theme.color.focus_ring),
            padding: Insets::ZERO,
            spacing: theme.space(1.0),
            min_height: MIN_HIT_TARGET,
            equal_widths: false,
            tab_corners: theme.corners(theme.radius.sm),
            tab_padding: Insets::symmetric(theme.space(3.0), theme.space(1.5)),
            hover: theme.color.surface_hover,
            pressed: theme.color.surface_pressed,
            label: theme.color.secondary_label,
            selected_label: theme.color.label,
            disabled_label: theme.color.disabled_label,
            label_size: theme.typography.body_size,
        };

        match variant {
            TabsVariant::Segmented => {
                let sumur = theme.space(0.5);
                let dalam = (theme.radius.md - sumur).max(0.0);
                Self {
                    track: Decoration::fill(theme.color.surface_sunken)
                        .corners(theme.corners(theme.radius.md))
                        .border(rambut, theme.color.separator),
                    indicator: Decoration::fill(theme.color.surface_elevated)
                        .corners(theme.corners(dalam))
                        .border(rambut, theme.color.separator)
                        .shadows(theme.shadow.sm),
                    tab_corners: theme.corners(dalam),
                    padding: Insets::all(sumur),
                    spacing: 0.0,
                    equal_widths: true,
                    ..dasar
                }
            }
            TabsVariant::Underline => Self {
                rail: Some(theme.color.separator),
                indicator: Decoration::fill(theme.color.accent)
                    .corners(theme.corners(theme.space(0.25))),
                spacing: theme.space(1.0),
                ..dasar
            },
            TabsVariant::Enclosed => Self {
                rail: Some(theme.color.separator),
                indicator: Decoration::fill(theme.color.surface_elevated)
                    .corners(Corners::new(
                        CornerRadii {
                            top_left: theme.radius.md,
                            top_right: theme.radius.md,
                            bottom_right: 0.0,
                            bottom_left: 0.0,
                        },
                        theme.radius.style,
                    ))
                    .border(rambut, theme.color.separator),
                spacing: theme.space(0.5),
                ..dasar
            },
        }
    }

    /// Kotak indikator untuk tab yang kotaknya `tab` (koordinat lokal deretan).
    ///
    /// Inilah satu-satunya tempat ketiga varian berbeda secara geometri —
    /// dan karena ia fungsi murni, seluruh perbedaan itu bisa diuji tanpa
    /// menyentuh render tree.
    pub fn indicator_rect(&self, tab: Rect) -> Rect {
        let kotak = tab.deflate(self.indicator_inset);
        match self.variant {
            TabsVariant::Segmented | TabsVariant::Enclosed => kotak,
            TabsVariant::Underline => {
                let tebal = self.indicator_thickness.min(kotak.size.height);
                Rect::new(
                    kotak.min_x(),
                    kotak.max_y() - tebal,
                    kotak.size.width,
                    tebal,
                )
            }
        }
    }

    /// Benar bila indikator menyumbang piksel sama sekali.
    pub fn indicator_is_visible(&self) -> bool {
        self.indicator.is_visible()
    }

    /// Bayangan indikator (kosong untuk varian tanpa elevasi).
    pub fn indicator_shadows(&self) -> ShadowPair {
        self.indicator.shadows
    }
}
