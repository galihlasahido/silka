//! The gallery's **page catalogue**: which demo pages exist, what they are
//! called, where they sit in `KOMPONEN.md`'s tier list, and how to build each
//! one.
//!
//! This module exists so that adding a component to the gallery is a
//! **one-line change**: add a variant, give it a slug, a title and a tier, and
//! point [`Halaman::view`] at its page function. The sidebar, the `--page`
//! command line argument, and the tests all read from here, so a page can
//! never be reachable through one of them and invisible to the others — the
//! failure mode that turns a gallery into a graveyard of forgotten demos
//! (REKOMENDASI §9.9).
//!
//! Two things deliberately do **not** live here:
//!
//! - the pages themselves — each one is its own module, and this file only
//!   knows the name of their `halaman()` function;
//! - anything visual — grouping is a *fact about the catalogue*
//!   (`KOMPONEN.md`), not a design decision, so no color or size appears
//!   anywhere in this file.

use silka_core::app::BuildCtx;
use silka_core::view::View;
use silka_widgets::Fonts;

/// Where a page sits in the `KOMPONEN.md` tier list.
///
/// This is what the sidebar groups by, and the order of [`Kelompok::SEMUA`] is
/// the order the groups appear in — a reader scrolling the sidebar walks the
/// catalogue in dependency order, exactly as the document does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kelompok {
    /// The reactive lifecycle itself: pages that prove the machinery, not a
    /// component.
    Fondasi,
    /// Tier 0 — primitives (text, container, corners, shadows).
    Primitif,
    /// Tier 1 — layout and scrolling.
    Layout,
    /// Tier 2 — the basic controls every application needs.
    Kontrol,
    /// Tier 3 — navigation and application structure.
    Navigasi,
    /// Tier 4 — overlays and feedback.
    Overlay,
    /// Tier 5 — data display.
    Data,
    /// Tier 6 — advanced components driven by a flagship application's needs.
    Lanjutan,
    /// Motion: the spring system the whole design system transitions on.
    Gerak,
}

impl Kelompok {
    /// Every group, in sidebar order.
    pub const SEMUA: [Kelompok; 9] = [
        Kelompok::Fondasi,
        Kelompok::Primitif,
        Kelompok::Layout,
        Kelompok::Kontrol,
        Kelompok::Navigasi,
        Kelompok::Overlay,
        Kelompok::Data,
        Kelompok::Lanjutan,
        Kelompok::Gerak,
    ];

    /// The group heading shown in the sidebar.
    pub fn judul(self) -> &'static str {
        match self {
            Kelompok::Fondasi => "Fondasi",
            Kelompok::Primitif => "Tier 0 · Primitif",
            Kelompok::Layout => "Tier 1 · Layout",
            Kelompok::Kontrol => "Tier 2 · Kontrol",
            Kelompok::Navigasi => "Tier 3 · Navigasi",
            Kelompok::Overlay => "Tier 4 · Overlay",
            Kelompok::Data => "Tier 5 · Data",
            Kelompok::Lanjutan => "Tier 6 · Lanjutan",
            Kelompok::Gerak => "Gerak",
        }
    }
}

/// One demo page of the gallery.
///
/// Every variant is a page that lives **inside the shell** — it hands back a
/// view tree and is therefore driven by the reactive lifecycle. The two older
/// pages that assemble a `Scene` by hand (the typography specimen and the
/// corner comparison) are not part of this enum; they hang off
/// `HalamanScene` in `main.rs` and are reachable only through `--page`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Halaman {
    /// Tier 0: the type scale, corner shapes, and layered shadows as views.
    Primitif,
    /// Tier 0/1: spacer, divider, stack, align/center, aspect ratio, image and
    /// icon — the primitives every other page is quietly built out of.
    TataLetak,
    /// The end-to-end counter — the smallest complete proof of the lifecycle.
    Counter,
    /// The card grid driven entirely through the reactive lifecycle.
    Reaktif,
    /// The utility vocabulary of §2.6 as a living reference.
    Utility,
    /// Tier 2: the `button` catalogue (five variants, every state).
    Tombol,
    /// Tier 2: the `text_field` catalogue (caret, selection, IME).
    KolomTeks,
    /// Tier 2: the `text_area` catalogue (soft wrap, goal column, gutter).
    AreaTeks,
    /// Tier 2: the `checkbox` catalogue (including indeterminate).
    Centang,
    /// Tier 2: the `switch` catalogue (a thumb you can drag).
    Sakelar,
    /// Tier 2: the `slider` catalogue (steps, range, keyboard).
    Slider,
    /// Tier 2: the `select` catalogue (anchored popup, typeahead).
    Pilihan,
    /// Tier 3: the `tabs` catalogue (three variants on one engine).
    Tabs,
    /// Tier 3: the in-app `menu` catalogue (dropdown + context menu).
    MenuHalaman,
    /// Tier 4: modal dialogs and alerts on the overlay system.
    Dialog,
    /// Tier 1: `scroll_view` — rubber banding, momentum, overlay scrollbars.
    Gulir,
    /// Tier 1: the **virtualized** list.
    Daftar,
    /// Tier 5: the **virtualized** table.
    Tabel,
    /// Tier 5: the **virtualized** tree (outline view).
    Pohon,
    /// Tier 5: the `silka-chart` catalogue.
    Chart,
    /// Tier 6: the rich text editor.
    Wysiwyg,
    /// The interactive spring playground.
    Animasi,
}

impl Halaman {
    /// Every page, in sidebar order (grouped by tier).
    pub const SEMUA: [Halaman; 22] = [
        Halaman::Counter,
        Halaman::Reaktif,
        Halaman::Utility,
        Halaman::Primitif,
        Halaman::TataLetak,
        Halaman::Gulir,
        Halaman::Daftar,
        Halaman::Tombol,
        Halaman::KolomTeks,
        Halaman::AreaTeks,
        Halaman::Centang,
        Halaman::Sakelar,
        Halaman::Slider,
        Halaman::Pilihan,
        Halaman::Tabs,
        Halaman::MenuHalaman,
        Halaman::Dialog,
        Halaman::Tabel,
        Halaman::Pohon,
        Halaman::Chart,
        Halaman::Wysiwyg,
        Halaman::Animasi,
    ];

    /// The page that opens when the gallery is started without arguments.
    pub const AWAL: Halaman = Halaman::Primitif;

    /// The canonical name used by `--page` and as the component key of the
    /// content area.
    ///
    /// Because it is also the component key, two pages sharing a slug would
    /// share their state as well — which is why a test pins them to be unique.
    pub fn slug(self) -> &'static str {
        match self {
            Halaman::Primitif => "primitives",
            Halaman::TataLetak => "layout",
            Halaman::Counter => "counter",
            Halaman::Reaktif => "reactive",
            Halaman::Utility => "utility",
            Halaman::Tombol => "button",
            Halaman::KolomTeks => "text-field",
            Halaman::AreaTeks => "text-area",
            Halaman::Centang => "checkbox",
            Halaman::Sakelar => "switch",
            Halaman::Slider => "slider",
            Halaman::Pilihan => "select",
            Halaman::Tabs => "tabs",
            Halaman::MenuHalaman => "menu",
            Halaman::Dialog => "dialog",
            Halaman::Gulir => "scroll",
            Halaman::Daftar => "list",
            Halaman::Tabel => "table",
            Halaman::Pohon => "tree",
            Halaman::Chart => "chart",
            Halaman::Wysiwyg => "wysiwyg",
            Halaman::Animasi => "spring",
        }
    }

    /// The label shown in the sidebar — also the a11y name of its nav button,
    /// so what a test clicks is exactly what a screen reader announces (§3.8).
    pub fn judul(self) -> &'static str {
        match self {
            Halaman::Primitif => "Teks & kontainer",
            Halaman::TataLetak => crate::layout::JUDUL,
            Halaman::Counter => "Counter",
            Halaman::Reaktif => "Grid reaktif",
            Halaman::Utility => "Kosakata utility",
            Halaman::Tombol => "Button",
            Halaman::KolomTeks => "Text field",
            Halaman::AreaTeks => "Text area",
            Halaman::Centang => "Checkbox",
            Halaman::Sakelar => "Switch",
            Halaman::Slider => "Slider",
            Halaman::Pilihan => "Select",
            Halaman::Tabs => "Tabs",
            Halaman::MenuHalaman => "Menu & context menu",
            Halaman::Dialog => "Dialog & alert",
            Halaman::Gulir => "Scroll view",
            Halaman::Daftar => "List (virtual)",
            Halaman::Tabel => "Table (virtual)",
            Halaman::Pohon => "Tree (virtual)",
            Halaman::Chart => "Chart",
            Halaman::Wysiwyg => "WYSIWYG editor",
            Halaman::Animasi => "Spring",
        }
    }

    /// The tier this page belongs to.
    pub fn kelompok(self) -> Kelompok {
        match self {
            Halaman::Counter | Halaman::Reaktif | Halaman::Utility => Kelompok::Fondasi,
            Halaman::Primitif => Kelompok::Primitif,
            Halaman::TataLetak | Halaman::Gulir | Halaman::Daftar => Kelompok::Layout,
            Halaman::Tombol
            | Halaman::KolomTeks
            | Halaman::AreaTeks
            | Halaman::Centang
            | Halaman::Sakelar
            | Halaman::Slider
            | Halaman::Pilihan => Kelompok::Kontrol,
            Halaman::Tabs | Halaman::MenuHalaman => Kelompok::Navigasi,
            Halaman::Dialog => Kelompok::Overlay,
            Halaman::Tabel | Halaman::Pohon | Halaman::Chart => Kelompok::Data,
            Halaman::Wysiwyg => Kelompok::Lanjutan,
            Halaman::Animasi => Kelompok::Gerak,
        }
    }

    /// True when the page manages its own vertical space and must therefore be
    /// handed a **bounded** box.
    ///
    /// The shell wraps every other page in a `scroll_view` so that a long
    /// catalogue is still reachable in a small window. Three kinds of page may
    /// not be wrapped:
    ///
    /// - pages that scroll already (`scroll_view`, `list`, `table`, `chart`) —
    ///   nesting two scroll containers on the same axis is a usability bug, not
    ///   a feature;
    /// - pages built out of `expanded()` (the reactive grid), because a
    ///   scrolling parent hands down an unbounded main axis and a flex child
    ///   cannot divide up infinity;
    /// - pages with an overlay layer (dialog, select), whose panel would be
    ///   **clipped** by the viewport it sits inside.
    pub fn gulir_sendiri(self) -> bool {
        matches!(
            self,
            Halaman::Gulir
                | Halaman::Daftar
                | Halaman::Tabel
                | Halaman::Pohon
                | Halaman::Chart
                | Halaman::Reaktif
                | Halaman::Dialog
                | Halaman::Pilihan
                | Halaman::MenuHalaman
                | Halaman::Wysiwyg
        )
    }

    /// Resolve a `--page` argument, including the aliases the older gallery
    /// accepted (both Indonesian and English spellings).
    pub fn dari_nama(nama: &str) -> Option<Halaman> {
        Some(match nama {
            "primitives" | "primitif" | "dasar" => Halaman::Primitif,
            "layout" | "tata-letak" | "tataletak" | "media" => Halaman::TataLetak,
            "counter" | "pencacah" => Halaman::Counter,
            "reactive" | "reaktif" => Halaman::Reaktif,
            "utility" | "utilitas" | "kosakata" => Halaman::Utility,
            "button" | "tombol" => Halaman::Tombol,
            "text-field" | "text_field" | "kolom-teks" => Halaman::KolomTeks,
            "text-area" | "text_area" | "area-teks" | "textarea" => Halaman::AreaTeks,
            "checkbox" | "centang" => Halaman::Centang,
            "switch" | "toggle" | "sakelar" => Halaman::Sakelar,
            "slider" | "penggeser" => Halaman::Slider,
            "select" | "dropdown" | "pilihan" => Halaman::Pilihan,
            "tabs" | "tab" => Halaman::Tabs,
            "menu" | "context-menu" | "context_menu" | "menu-konteks" => Halaman::MenuHalaman,
            "dialog" | "alert" => Halaman::Dialog,
            "scroll" | "scroll_view" | "gulir" => Halaman::Gulir,
            "list" | "daftar" => Halaman::Daftar,
            "table" | "tabel" => Halaman::Tabel,
            "tree" | "pohon" | "outline" => Halaman::Pohon,
            "chart" | "grafik" | "bagan" => Halaman::Chart,
            "wysiwyg" | "editor" | "rich-text" | "teks-kaya" => Halaman::Wysiwyg,
            "spring" | "animasi" | "animation" => Halaman::Animasi,
            _ => return None,
        })
    }

    /// Build this page's view tree.
    ///
    /// The single place that maps a catalogue entry to a page module. Every
    /// arm has the same shape on purpose: a page is a plain function of
    /// `(&BuildCtx, &Fonts)`, so the shell needs no per-page special case.
    pub fn view(self, cx: &BuildCtx, fonts: &Fonts) -> View {
        match self {
            Halaman::Primitif => crate::primitives::halaman(cx, fonts),
            Halaman::TataLetak => crate::layout::halaman(cx, fonts),
            Halaman::Counter => crate::counter::halaman(cx, fonts),
            Halaman::Reaktif => crate::reactive::halaman(cx),
            Halaman::Utility => crate::utility::halaman(cx, fonts),
            Halaman::Tombol => crate::button::halaman(cx, fonts),
            Halaman::KolomTeks => crate::text_field::halaman(cx, fonts),
            Halaman::AreaTeks => crate::text_area::halaman(cx, fonts),
            Halaman::Centang => crate::checkbox::halaman(cx, fonts),
            Halaman::Sakelar => crate::switch::halaman(cx, fonts),
            Halaman::Slider => crate::slider::halaman(cx, fonts),
            Halaman::Pilihan => crate::select::halaman(cx, fonts),
            Halaman::Tabs => crate::tabs::halaman(cx, fonts),
            Halaman::MenuHalaman => crate::menu::halaman(cx, fonts),
            Halaman::Dialog => crate::dialog::halaman(cx, fonts),
            Halaman::Gulir => crate::scroll_view::halaman(cx, fonts),
            Halaman::Daftar => crate::list::halaman(cx, fonts),
            Halaman::Tabel => crate::table::halaman(cx, fonts),
            Halaman::Pohon => crate::tree::halaman(cx, fonts),
            Halaman::Chart => crate::chart::halaman(cx, fonts),
            Halaman::Wysiwyg => crate::wysiwyg::halaman(cx, fonts),
            Halaman::Animasi => crate::spring::halaman(cx, fonts),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn setiap_halaman_punya_slug_unik() {
        let mut lihat = HashSet::new();
        for h in Halaman::SEMUA {
            assert!(
                lihat.insert(h.slug()),
                "slug '{}' dipakai dua halaman — keduanya akan berbagi state \
                 karena slug juga dipakai sebagai kunci komponen",
                h.slug()
            );
        }
    }

    #[test]
    fn setiap_halaman_punya_judul_unik() {
        let mut lihat = HashSet::new();
        for h in Halaman::SEMUA {
            assert!(
                lihat.insert(h.judul()),
                "judul '{}' dipakai dua halaman: tombol navigasinya tidak bisa \
                 dibedakan pembaca layar",
                h.judul()
            );
        }
    }

    /// The sidebar only renders pages whose group is in `Kelompok::SEMUA`, so a
    /// page whose group is missing from that list would silently disappear.
    #[test]
    fn setiap_halaman_masuk_salah_satu_kelompok_sidebar() {
        for h in Halaman::SEMUA {
            assert!(
                Kelompok::SEMUA.contains(&h.kelompok()),
                "{} tidak akan pernah tampil di sidebar",
                h.judul()
            );
        }
    }

    #[test]
    fn setiap_kelompok_terpakai() {
        for k in Kelompok::SEMUA {
            assert!(
                Halaman::SEMUA.iter().any(|h| h.kelompok() == k),
                "kelompok '{}' kosong",
                k.judul()
            );
        }
    }

    #[test]
    fn slug_bisa_dipakai_sebagai_argumen() {
        for h in Halaman::SEMUA {
            assert_eq!(
                Halaman::dari_nama(h.slug()),
                Some(h),
                "--page {} tidak dikenali",
                h.slug()
            );
        }
    }

    #[test]
    fn alias_lama_masih_dikenali() {
        // The aliases the gallery accepted before the shell existed; breaking
        // them would break every note and script that already uses them.
        for (alias, halaman) in [
            ("tombol", Halaman::Tombol),
            ("centang", Halaman::Centang),
            ("sakelar", Halaman::Sakelar),
            ("toggle", Halaman::Sakelar),
            ("pilihan", Halaman::Pilihan),
            ("dropdown", Halaman::Pilihan),
            ("gulir", Halaman::Gulir),
            ("daftar", Halaman::Daftar),
            ("tabel", Halaman::Tabel),
            ("kolom-teks", Halaman::KolomTeks),
            ("pencacah", Halaman::Counter),
            ("reaktif", Halaman::Reaktif),
            ("tab", Halaman::Tabs),
            ("alert", Halaman::Dialog),
            ("bagan", Halaman::Chart),
        ] {
            assert_eq!(Halaman::dari_nama(alias), Some(halaman), "alias {alias}");
        }
    }

    #[test]
    fn nama_asing_ditolak_bukan_diam_diam_jadi_halaman_lain() {
        assert_eq!(Halaman::dari_nama("ngawur"), None);
        assert_eq!(Halaman::dari_nama(""), None);
    }
}
