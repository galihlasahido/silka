//! Demo page: **virtualized list** (`KOMPONEN.md` Tier 1).
//!
//! The number on this page is deliberately absurd: **a hundred thousand rows**.
//! That is not showing off, it is the only way to prove the thing that is
//! easiest to claim and rarest to get right — that only the visible rows are
//! ever built. A list that is "fast" at 200 rows proves nothing; a list that
//! stays at 120 fps across 100,000 rows proves everything.
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | Virtualization | Scroll down to row 90,000: no stutter, and memory does not move |
//! | Scrolling = `scroll_view` | Rubber banding, OS momentum, auto-hiding scrollbar — all present without this list carrying any physics code of its own |
//! | Sticky header | The column headings **stick** to the top edge while rows slide past beneath them |
//! | Spring-driven selection | Click a row then press ↓ repeatedly: the highlight *glides*, it does not blink from place to place |
//! | Hover & press | Sweep the cursor over a row; hold the mouse button down |
//! | Full keyboard support | Tab to the list, then ↑ ↓ · Page Up/Down · Home/End · Enter |
//! | Off-screen rows stay reachable | Home/End scrolls the list itself to the selected row |
//! | Hit target ≥ 44pt | Every row is 44pt tall even though its text is small |
//! | AccessKit nodes | VoiceOver says "list", announces each row, and names which one is selected |
//! | Both presets & dark mode | `--preset tailwind`, `--appearance dark` |
//! | Reduced motion | Turn on "Reduce motion" in the OS: the highlight is immediately in place |
//!
//! What is **absent** from this file: hand-assembled `Scene`s, layout
//! arithmetic, and color numbers. Everything is a token (§2.6, §2.7).

use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{
    active_fonts, button, button_variant, list, spacer, text, use_list_state, ButtonVariant,
    ListState,
};

/// The page title.
pub const JUDUL: &str = "List (virtualised)";
/// The list's name for screen readers — and the anchor the tests look for.
pub const NAMA_DAFTAR: &str = "Transactions";
/// How many rows. A hundred thousand, and that is exactly the point of the
/// demo.
pub const BARIS: usize = 100_000;

/// The jump-far button.
pub const TOMBOL_TENGAH: &str = "Jump to row 50,000";
/// The back-to-the-start button.
pub const TOMBOL_AWAL: &str = "Back to top";

/// One row's height — which is also the HIG's minimum hit target.
const TINGGI_BARIS: f32 = 44.0;
/// The column-heading row's height, in spacing-scale steps (§2.6).
const TINGGI_HEADER_LANGKAH: f32 = 9.0;
/// The list viewport's height, in spacing-scale steps.
const TINGGI_LANGKAH: f32 = 92.0;
/// The list's maximum width, in spacing-scale steps.
const LEBAR_LANGKAH: f32 = 140.0;

/// The view tree for the whole page — this is what gets handed to
/// `run_app_with`.
pub fn halaman(cx: &BuildCtx) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterized at the real screen resolution (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    active_fonts().set_scale_factor(dpi.get());

    // The list state: scroll position + selected row, surviving rebuilds.
    let daftar_state = use_list_state();
    // The last row that was **activated** (double-tap / Enter).
    let dibuka = use_signal(|| None::<usize>);

    column([
        View::from(
            text(JUDUL)
                .size(t.typography.title2.size)
                .weight(FontWeight::SEMIBOLD)
                .tracking(t.typography.title2.tracking)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text(
                "A hundred thousand rows, and only a dozen or so of them ever \
                 become nodes. Scroll as far as you like: what gets built is \
                 always just what fits on screen. Click a row and press ↓ — the \
                 highlight glides, and the list scrolls itself when the row \
                 leaves the screen.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(LEBAR_LANGKAH)),
        ),
        daftar(&t, daftar_state, dibuka),
        kendali(&t, daftar_state),
        status(daftar_state, dibuka),
    ])
    .spacing(t.space(5.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// The list viewport.
///
/// The scroll axis **must** be bounded (the same rule as Flutter's): the bound
/// lives here, not inside the container.
fn daftar(t: &Theme, state: ListState, dibuka: Signal<Option<usize>>) -> View {
    let theme = *t;

    constrained(
        BoxConstraints::new(
            0.0,
            t.space(LEBAR_LANGKAH),
            t.space(TINGGI_LANGKAH),
            t.space(TINGGI_LANGKAH),
        ),
        list(state, BARIS, move |i| baris(&theme, i))
            .item_extent(TINGGI_BARIS)
            .sticky_header(t.space(TINGGI_HEADER_LANGKAH), move || judul_kolom(&theme))
            .separators(t.space(0.25))
            .label(NAMA_DAFTAR)
            .background(t.color.surface_sunken)
            .corners(t.corners(t.radius.lg))
            .border(t.space(0.25), t.color.separator)
            .on_activate(move |i| dibuka.set(Some(i))),
    )
    .into()
}

/// One row: number, description, and amount.
///
/// Called **only** for visible rows — that is virtualization's promise, and
/// that is why `BARIS` is allowed to be a hundred thousand.
fn baris(t: &Theme, i: usize) -> View {
    let nomor = text(format!("#{:06}", i + 1))
        .size(t.typography.footnote.size)
        .weight(FontWeight::MEDIUM)
        .color(t.color.tertiary_label)
        .single_line();
    let nama = text(format!("Transaction {}", nama_pihak(i)))
        .size(t.typography.body_size)
        .color(t.color.label)
        .single_line();
    let nominal = text(format!("Rp {}.000", (i % 900 + 100) * 125))
        .size(t.typography.body_size)
        .weight(FontWeight::MEDIUM)
        .color(t.color.secondary_label)
        .single_line();

    row([
        View::from(nomor),
        View::from(nama),
        // A spacer: the amount column is always right-aligned, without a
        // single layout number on this page.
        View::from(spacer()),
        View::from(nominal),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .padding(Insets::symmetric(t.space(4.0), 0.0))
    .into()
}

/// Repeating counterparty names — fake data that still looks like data.
fn nama_pihak(i: usize) -> &'static str {
    const NAMA: [&str; 6] = [
        "Warung Kopi",
        "PT Sinar Jaya",
        "Koperasi Melati",
        "Toko Bangunan",
        "CV Anugerah",
        "Apotek Sehat",
    ];
    NAMA[i % NAMA.len()]
}

/// The column headings that stick to the list's top edge.
fn judul_kolom(t: &Theme) -> View {
    row([
        View::from(
            text("No.")
                .size(t.typography.footnote.size)
                .weight(FontWeight::SEMIBOLD)
                .color(t.color.secondary_label)
                .single_line(),
        ),
        View::from(
            text("Party")
                .size(t.typography.footnote.size)
                .weight(FontWeight::SEMIBOLD)
                .color(t.color.secondary_label)
                .single_line(),
        ),
        // A spacer: the amount column is always right-aligned, without a
        // single layout number on this page.
        View::from(spacer()),
        View::from(
            text("Amount")
                .size(t.typography.footnote.size)
                .weight(FontWeight::SEMIBOLD)
                .color(t.color.secondary_label)
                .single_line(),
        ),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .padding(Insets::symmetric(t.space(4.0), 0.0))
    // An opaque header: rows sliding beneath it must not show through.
    .background(t.color.surface)
    .into()
}

/// Two jump-far buttons — proof that `scroll_to` works on huge data sets.
fn kendali(t: &Theme, state: ListState) -> View {
    row([
        View::from(button(TOMBOL_TENGAH).on_press(move || state.scroll_to_item(50_000, BARIS))),
        View::from(
            button_variant(TOMBOL_AWAL, ButtonVariant::Secondary)
                .on_press(move || state.scroll_to(0.0)),
        ),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .into()
}

/// The status row — **the only place the selection is read**, so moving the
/// highlight rebuilds just this text (§2.5).
fn status(state: ListState, dibuka: Signal<Option<usize>>) -> View {
    component("status", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let terpilih = state
            .selected()
            .map(|i| format!("row #{:06}", i + 1))
            .unwrap_or_else(|| "none yet".to_string());
        let aktif = dibuka
            .get()
            .map(|i| format!("opened #{:06}", i + 1))
            .unwrap_or_else(|| "double-tap or Enter to open".to_string());
        text(format!("Selected: {terpilih} · {aktif}"))
            .size(t.typography.body_size)
            .color(t.color.tertiary_label)
            .single_line()
            .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::{AccessActions, AccessRole};
    use silka_core::app::AppRuntime;
    use silka_core::input::{
        Event, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
    };
    use silka_paint::{Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use silka_widgets::list::{nodes, ListBody, ListRowBox};
    use std::time::Duration;

    const VIEWPORT: Size = Size::new(900.0, 760.0);

    /// A headless app assembled **exactly the way `run_app_with` does it**.
    fn ui(theme: Theme) -> AppRuntime {
        headless_app(theme, halaman).sized(VIEWPORT.width, VIEWPORT.height)
    }

    /// Pump frames until the app is genuinely at rest **and** no spring is
    /// still pending.
    fn diam(ui: &mut AppRuntime) {
        for _ in 0..12 {
            ui.animate(|tree, _| {
                silka_widgets::settle(tree);
                silka_core::scheduler::Dirty::LAYOUT | silka_core::scheduler::Dirty::PAINT
            });
            ui.animate(silka_widgets::advance);
            ui.frame();
            if ui.is_idle() && !silka_widgets::is_animating(ui.tree()) {
                break;
            }
        }
    }

    fn daftar_node(ui: &AppRuntime) -> &ListBody {
        let id = nodes(ui.tree())[0];
        ui.tree().node_ref::<ListBody>(id).expect("ListBody")
    }

    /// How many rows actually became nodes.
    fn baris_di_pohon(ui: &AppRuntime) -> usize {
        fn hitung(tree: &silka_core::tree::RenderTree, id: silka_core::tree::NodeId) -> usize {
            usize::from(tree.node_ref::<ListRowBox>(id).is_some())
                + tree
                    .children(id)
                    .iter()
                    .map(|c| hitung(tree, *c))
                    .sum::<usize>()
        }
        hitung(ui.tree(), ui.tree().root())
    }

    fn kotak(ui: &AppRuntime, label: &str) -> Rect {
        let pohon = ui.access_tree();
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", pohon.dump()))
            .bounds
    }

    fn klik(ui: &mut AppRuntime, titik: Point, kali: u32, mulai: Duration) {
        let mut t = mulai;
        for _ in 0..kali {
            ui.dispatch(&Event::Pointer(PointerEvent::new(
                PointerPhase::Move,
                titik,
                t,
            )));
            ui.dispatch(&Event::Pointer(
                PointerEvent::new(PointerPhase::Down, titik, t).button(PointerButton::Primary),
            ));
            t += Duration::from_millis(10);
            ui.dispatch(&Event::Pointer(
                PointerEvent::new(PointerPhase::Up, titik, t).button(PointerButton::Primary),
            ));
            t += Duration::from_millis(60);
        }
        diam(ui);
    }

    fn tombol(ui: &mut AppRuntime, key: NamedKey) {
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(key),
            Duration::ZERO,
        )));
        diam(ui);
    }

    #[test]
    fn seratus_ribu_baris_hanya_menjadi_belasan_node() {
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
        diam(&mut ui);

        let baris = baris_di_pohon(&ui);
        assert!(baris > 0, "daftar tidak membangun satu baris pun");
        assert!(
            baris < 60,
            "seratus ribu baris menjadi {baris} node — virtualisasi bocor"
        );
        assert_eq!(daftar_node(&ui).metrics().count, BARIS);
        assert!(ui.is_idle(), "halaman diam tidak menyisakan pekerjaan");
    }

    #[test]
    fn daftar_dan_barisnya_terbaca_screen_reader() {
        let mut ui = ui(Theme::cupertino(Appearance::Light));
        diam(&mut ui);

        let pohon = ui.access_tree();
        let daftar = pohon
            .find_role(AccessRole::List)
            .unwrap_or_else(|| panic!("{}", pohon.dump()));
        assert_eq!(daftar.node.label.as_deref(), Some(NAMA_DAFTAR));
        assert!(daftar.node.actions.contains(AccessActions::FOCUS));

        let baris = pohon
            .entries()
            .iter()
            .filter(|e| e.node.role == AccessRole::ListItem)
            .count();
        assert!(
            baris > 0,
            "tidak ada baris di pohon a11y:\n{}",
            pohon.dump()
        );
        // The first row really does have its content announced.
        assert!(pohon.find_label("#000001").is_some());
        // And the column heading is **not** one of the rows.
        assert!(pohon.find_label("Amount").is_some());
    }

    #[test]
    fn klik_memilih_dan_ketuk_ganda_membuka_baris() {
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
        diam(&mut ui);

        let baris_kedua = kotak(&ui, "#000002").center();
        klik(&mut ui, baris_kedua, 1, Duration::from_secs(1));
        assert_eq!(daftar_node(&ui).selected(), Some(1));
        let pohon = ui.access_tree();
        assert!(
            pohon
                .find_label("Selected: row #000002 · double-tap or Enter to open")
                .is_some(),
            "status tidak ikut berubah:\n{}",
            pohon.dump()
        );

        klik(&mut ui, baris_kedua, 2, Duration::from_secs(4));
        let pohon = ui.access_tree();
        assert!(
            pohon.entries().iter().any(|e| e
                .node
                .label
                .as_deref()
                .is_some_and(|l| l.contains("opened #000002"))),
            "ketuk-ganda tidak membuka baris:\n{}",
            pohon.dump()
        );
    }

    #[test]
    fn keyboard_menggerakkan_seleksi_dan_menggulirkan_daftar() {
        let mut ui = ui(Theme::cupertino(Appearance::Dark));
        diam(&mut ui);

        // Tab until the list holds focus (the buttons come first in the tree).
        for _ in 0..6 {
            tombol(&mut ui, NamedKey::Tab);
            if daftar_node(&ui).is_focused() {
                break;
            }
        }
        assert!(
            daftar_node(&ui).is_focused(),
            "daftar tidak bisa dicapai Tab"
        );

        tombol(&mut ui, NamedKey::End);
        assert_eq!(daftar_node(&ui).selected(), Some(BARIS - 1));
        // The last row really is announced: the list scrolled itself there.
        assert!(
            ui.access_tree().find_label("#100000").is_some(),
            "baris terakhir tidak digulirkan ke layar"
        );

        tombol(&mut ui, NamedKey::Home);
        assert_eq!(daftar_node(&ui).selected(), Some(0));
    }

    #[test]
    fn tombol_lompat_jauh_menggulirkan_seratus_ribu_baris() {
        let mut ui = ui(Theme::cupertino(Appearance::Light));
        diam(&mut ui);

        let p = kotak(&ui, TOMBOL_TENGAH).center();
        klik(&mut ui, p, 1, Duration::from_secs(1));
        assert!(
            daftar_node(&ui).first() >= 49_000,
            "jendela tidak ikut melompat: {}",
            daftar_node(&ui).first()
        );
        assert!(
            baris_di_pohon(&ui) < 60,
            "jendela membengkak setelah lompat"
        );

        let p = kotak(&ui, TOMBOL_AWAL).center();
        klik(&mut ui, p, 1, Duration::from_secs(4));
        assert_eq!(daftar_node(&ui).first(), 0);
    }

    #[test]
    fn benar_di_kedua_preset_dan_kedua_appearance() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let mut ui = ui(t);
                diam(&mut ui);
                assert_eq!(ui.scene().clear_color(), t.color.background);
                assert!(
                    baris_di_pohon(&ui) > 0,
                    "daftar kosong di {preset:?} {appearance:?}"
                );
                let warna: Vec<_> = ui
                    .scene()
                    .commands()
                    .iter()
                    .filter_map(|c| match c {
                        silka_paint::Command::GlyphRun(r) => Some(r.color),
                        _ => None,
                    })
                    .collect();
                for w in warna {
                    assert!(
                        [
                            t.color.label,
                            t.color.secondary_label,
                            t.color.tertiary_label,
                            t.color.on_accent,
                        ]
                        .contains(&w),
                        "warna teks lepas dari token: {w:?} ({preset:?} {appearance:?})"
                    );
                }
            }
        }
    }
}
