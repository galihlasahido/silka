//! Demo page: **virtualized table** (`KOMPONEN.md` Tier 5).
//!
//! The number is as absurd as on the `daftar` page: **a hundred thousand
//! rows**, four columns. And that is exactly the point of the demo — a table
//! that is "fast" at two hundred rows proves nothing, while a table that stays
//! smooth across a hundred thousand rows **while** its columns are dragged,
//! resized, and sorted proves the virtualization does not leak on any path.
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | Virtualization | Scroll to row 90,000: no stutter, memory does not move |
//! | No second virtualization system | Scrolling, rubber banding, and the scrollbar belong to `scroll_view`; the row window belongs to `ListMetrics` — exactly as on the `daftar` page |
//! | Per-column sorting | Click a column heading; click again to reverse it. A hundred thousand rows are sorted **once**, then cached |
//! | Column resizing | Drag the boundary between headings; the cursor changes as you approach it |
//! | Column reordering | Drag a heading left/right; the drop indicator glides along |
//! | Multi-selection | Click, ⇧-click to extend a range, ⌘-click to pick individually, ⌘A for everything |
//! | Sticky header | The headings stick to the top edge while rows slide past |
//! | Auto vs fixed widths | "Pihak" stretches with the window; "No.", "Status", and "Nominal" do not |
//! | Custom cells | The "Status" column holds a colored badge, not text — a cell accepts any widget |
//! | Cell-wise keyboard navigation | Tab to the table, then ↑ ↓ (⇧ extends) · ← → move by **cell** · Page · Home/End · Esc · Enter |
//! | Empty state | The "Kosongkan" button — an empty table still shows its column headings |
//! | AccessKit nodes | VoiceOver says "table", announcing each row and its cells |
//! | Both presets & dark mode | `--preset tailwind`, `--appearance dark` |
//! | Reduced motion | Turn on "Reduce motion" in the OS: the highlight is immediately in place |
//!
//! What is **absent** from this file: hand-assembled `Scene`s, layout
//! arithmetic, and color numbers. Everything is a token (§2.6, §2.7).

use std::cell::RefCell;
use std::rc::Rc;

use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, pad, row, View};
use silka_paint::{Color, Insets};
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{
    button, button_variant, col, table, text, use_table_state, ButtonVariant, Column, Fonts,
    SortBy, SortDirection, TableState,
};

/// The page title.
pub const JUDUL: &str = "Table (tervirtualisasi)";
/// The table's name for screen readers — and the anchor the tests look for.
pub const NAMA_TABEL: &str = "Transaksi";
/// How many rows. A hundred thousand, and that is exactly the point of the
/// demo.
pub const BARIS: usize = 100_000;

/// The jump-far button.
pub const TOMBOL_TENGAH: &str = "Ke baris 50.000";
/// The button that empties the table (showing off the empty state).
pub const TOMBOL_KOSONG: &str = "Kosongkan";
/// The button that puts the content back.
pub const TOMBOL_ISI: &str = "Isi lagi";
/// The button that restores the original column widths & order.
pub const TOMBOL_RESET: &str = "Reset kolom";

/// The empty-state text.
pub const KOSONG: &str = "Belum ada transaksi";

/// One row's height — which is also the HIG's minimum hit target.
const TINGGI_BARIS: f32 = 44.0;
/// The column-heading row's height, in spacing-scale steps (§2.6).
const TINGGI_HEADER_LANGKAH: f32 = 9.0;
/// The table viewport's height, in spacing-scale steps.
const TINGGI_LANGKAH: f32 = 92.0;
/// The table's maximum width, in spacing-scale steps.
const LEBAR_LANGKAH: f32 = 200.0;

// ---------------------------------------------------------------------------
// Fake data that still looks like data
// ---------------------------------------------------------------------------

const NAMA: [&str; 6] = [
    "Warung Kopi",
    "PT Sinar Jaya",
    "Koperasi Melati",
    "Toko Bangunan",
    "CV Anugerah",
    "Apotek Sehat",
];

const STATUS: [&str; 3] = ["Lunas", "Tertunda", "Batal"];

/// The counterparty name for row `i`.
fn nama_pihak(i: usize) -> &'static str {
    NAMA[(i * 7 + i / 6) % NAMA.len()]
}

/// The status of row `i`.
fn status(i: usize) -> &'static str {
    STATUS[(i * 3 + i / 5) % STATUS.len()]
}

/// The amount for row `i`, in rupiah.
fn nominal(i: usize) -> u64 {
    ((i * 8_191) % 900 + 100) as u64 * 125_000
}

/// An amount with thousands separators — an unseparated number is unreadable.
fn rupiah(n: u64) -> String {
    let angka = n.to_string();
    let mut out = String::with_capacity(angka.len() + angka.len() / 3);
    for (i, c) in angka.chars().enumerate() {
        if i > 0 && (angka.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(c);
    }
    format!("Rp {out}")
}

// ---------------------------------------------------------------------------
// Sorting: computed once, then cached
// ---------------------------------------------------------------------------

/// The row permutation produced by sorting, along with the key that produced
/// it.
///
/// This is not a luxury: sorting a hundred thousand rows on **every** rebuild
/// means every single pixel of scrolling pays O(n log n), and the
/// "virtualization" promise is broken somewhere nobody is looking. The cache
/// lives behind a [`RefCell`] rather than a signal precisely so that filling it
/// does **not** schedule a frame — it is derived data, not state.
#[derive(Default)]
struct Urutan {
    kunci: Option<Option<SortBy>>,
    baris: Rc<Vec<u32>>,
}

impl Urutan {
    /// The permutation for `sort`, recomputed only when its key changes.
    fn untuk(&mut self, sort: Option<SortBy>, count: usize) -> Rc<Vec<u32>> {
        if self.kunci == Some(sort) && self.baris.len() == count {
            return self.baris.clone();
        }
        let mut baris: Vec<u32> = (0..count as u32).collect();
        if let Some(s) = sort {
            baris.sort_by(|a, b| {
                let (a, b) = (*a as usize, *b as usize);
                let urut = match s.column {
                    0 => a.cmp(&b),
                    1 => nama_pihak(a).cmp(nama_pihak(b)).then(a.cmp(&b)),
                    2 => status(a).cmp(status(b)).then(a.cmp(&b)),
                    _ => nominal(a).cmp(&nominal(b)).then(a.cmp(&b)),
                };
                if s.direction == SortDirection::Descending {
                    urut.reverse()
                } else {
                    urut
                }
            });
        }
        let baris = Rc::new(baris);
        self.kunci = Some(sort);
        self.baris = baris.clone();
        baris
    }
}

// ---------------------------------------------------------------------------
// The page
// ---------------------------------------------------------------------------

/// The column definitions — the only place widths, alignment, and headings are
/// written.
pub fn kolom(t: &Theme) -> Vec<Column> {
    vec![
        col("No.").fixed(t.space(24.0)).min_width(t.space(16.0)),
        col("Pihak").flex(3.0).min_width(t.space(24.0)),
        col("Status").fixed(t.space(30.0)).center(),
        col("Nominal").fixed(t.space(38.0)).trailing(),
    ]
}

/// The view tree for the whole page — this is what gets handed to
/// `run_app_with`.
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterized at the real screen resolution (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    let tabel_state = use_table_state();
    let dibuka = use_signal(|| None::<usize>);
    let terisi = use_signal(|| true);
    // The sort permutation cache; see [`Urutan`].
    let urutan = use_signal(|| Rc::new(RefCell::new(Urutan::default())));

    column([
        View::from(
            text(fonts, JUDUL)
                .size(t.typography.title2.size)
                .weight(FontWeight::SEMIBOLD)
                .tracking(t.typography.title2.tracking)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text(
                fonts,
                "Seratus ribu baris berkolom, dan hanya belasan di antaranya \
                 yang pernah menjadi node — memakai ulang virtualisasi yang \
                 sama dengan komponen list, bukan sistem kedua. Klik judul \
                 kolom untuk mengurutkan, seret batasnya untuk melebarkan, \
                 seret judulnya untuk memindahkan. ⇧-klik merentang seleksi, \
                 ⌘-klik memungut satu per satu, ⌘A memilih semuanya.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(LEBAR_LANGKAH)),
        ),
        tabel(fonts, &t, tabel_state, dibuka, terisi, urutan),
        kendali(fonts, &t, tabel_state, terisi),
        status_bar(fonts, tabel_state, dibuka),
    ])
    .spacing(t.space(5.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// The table viewport.
///
/// The scroll axis **must** be bounded (the same rule as Flutter's): the bound
/// lives here, not inside the container.
fn tabel(
    fonts: &Fonts,
    t: &Theme,
    state: TableState,
    dibuka: Signal<Option<usize>>,
    terisi: Signal<bool>,
    urutan: Signal<Rc<RefCell<Urutan>>>,
) -> View {
    let count = if terisi.get() { BARIS } else { 0 };
    // Reading `sort()` here is what makes the table rebuild every time a
    // column heading is clicked — no callback needs wiring up (§2.5).
    let permutasi = urutan.peek().borrow_mut().untuk(state.sort(), count);

    let untuk_sel = fonts.clone();
    let untuk_kosong = fonts.clone();
    let theme = *t;

    constrained(
        BoxConstraints::new(
            0.0,
            t.space(LEBAR_LANGKAH),
            t.space(TINGGI_LANGKAH),
            t.space(TINGGI_LANGKAH),
        ),
        table(fonts, t, state, kolom(t), count, move |baris, kolom| {
            let data = permutasi[baris] as usize;
            sel(&untuk_sel, &theme, data, kolom)
        })
        .row_extent(TINGGI_BARIS)
        .header_extent(t.space(TINGGI_HEADER_LANGKAH))
        .separators(t.space(0.25))
        .striped()
        .label(NAMA_TABEL)
        .background(t.color.surface_sunken)
        .corners(t.corners(t.radius.lg))
        .border(t.space(0.25), t.color.separator)
        .empty(move || kosong(&untuk_kosong, &theme))
        .on_activate(move |i| dibuka.set(Some(i))),
    )
    .into()
}

/// One cell: its column decides its shape.
///
/// The "Status" column returns a **badge**, not text — that is what "custom
/// cells" means in `KOMPONEN.md`: a cell accepts any view, there is no special
/// cell type to learn.
fn sel(fonts: &Fonts, t: &Theme, i: usize, kolom: usize) -> View {
    match kolom {
        0 => text(fonts, format!("#{:06}", i + 1))
            .size(t.typography.footnote.size)
            .weight(FontWeight::MEDIUM)
            .color(t.color.tertiary_label)
            .single_line()
            .into(),
        1 => text(fonts, nama_pihak(i))
            .size(t.typography.body_size)
            .color(t.color.label)
            .single_line()
            .into(),
        2 => badge(fonts, t, status(i)),
        _ => text(fonts, rupiah(nominal(i)))
            .size(t.typography.body_size)
            .weight(FontWeight::MEDIUM)
            .color(t.color.secondary_label)
            .single_line()
            .into(),
    }
}

/// The status badge — the only "graphic" on this page, and all of its colors
/// are still tokens.
fn badge(fonts: &Fonts, t: &Theme, status: &str) -> View {
    let (latar, tulisan): (Color, Color) = match status {
        "Lunas" => (t.color.success, t.color.on_accent),
        "Tertunda" => (t.color.warning, t.color.on_accent),
        _ => (t.color.surface_pressed, t.color.secondary_label),
    };
    pad(
        Insets::symmetric(t.space(2.0), t.space(1.0)),
        text(fonts, status)
            .size(t.typography.footnote.size)
            .weight(FontWeight::SEMIBOLD)
            .color(tulisan)
            .single_line(),
    )
    .background(latar)
    .corners(t.corners(t.radius.sm))
    .into()
}

/// What shows up when the table is empty.
fn kosong(fonts: &Fonts, t: &Theme) -> View {
    column([View::from(
        text(fonts, KOSONG)
            .size(t.typography.body_size)
            .color(t.color.tertiary_label)
            .single_line(),
    )])
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .into()
}

/// The buttons that prove `scroll_to`, the empty state, and the column reset.
fn kendali(fonts: &Fonts, t: &Theme, state: TableState, terisi: Signal<bool>) -> View {
    let isi = terisi.get();
    let label_isi = if isi { TOMBOL_KOSONG } else { TOMBOL_ISI };
    row([
        View::from(
            button(fonts, t, TOMBOL_TENGAH).on_press(move || state.scroll_to_row(50_000, BARIS)),
        ),
        View::from(
            button_variant(fonts, t, label_isi, ButtonVariant::Secondary).on_press(move || {
                terisi.set(!isi);
                state.clear_selection();
            }),
        ),
        View::from(
            button_variant(fonts, t, TOMBOL_RESET, ButtonVariant::Secondary).on_press(move || {
                state.reset_widths();
                state.set_order(Vec::new());
                state.set_sort(None);
            }),
        ),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .into()
}

/// The status row — **the only place the selection is read**, so moving the
/// highlight rebuilds just this text (§2.5).
fn status_bar(fonts: &Fonts, state: TableState, dibuka: Signal<Option<usize>>) -> View {
    let fonts = fonts.clone();
    component("status-tabel", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let sel = state.selection();
        let terpilih = match sel.len() {
            0 => "belum ada".to_string(),
            1 => format!("baris #{:06}", sel.first().unwrap_or(0) + 1),
            n => format!("{n} baris"),
        };
        let urut = match state.sort() {
            Some(s) => format!(
                "urut kolom {} {}",
                s.column,
                if s.direction == SortDirection::Ascending {
                    "naik"
                } else {
                    "turun"
                }
            ),
            None => "tanpa urutan".to_string(),
        };
        let aktif = dibuka
            .get()
            .map(|i| format!("dibuka #{:06}", i + 1))
            .unwrap_or_else(|| "ketuk-ganda atau Enter untuk membuka".to_string());
        text(&fonts, format!("Terpilih: {terpilih} · {urut} · {aktif}"))
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
        Event, KeyCode, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent, PointerPhase,
    };
    use silka_paint::{Point, Rect, Size};
    use silka_platform::headless_app;
    use silka_theme::{Appearance, Preset};
    use silka_widgets::table::{header_nodes, nodes, TableBody, TableCellBox, TableHeaderBox};
    use std::time::Duration;

    const VIEWPORT: Size = Size::new(1200.0, 800.0);

    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    /// A headless app assembled **exactly the way `run_app_with` does it**.
    fn ui(theme: Theme, fonts: &Fonts) -> AppRuntime {
        let untuk_view = fonts.clone();
        headless_app(theme, move |cx| halaman(cx, &untuk_view))
            .sized(VIEWPORT.width, VIEWPORT.height)
    }

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

    fn body(ui: &AppRuntime) -> &TableBody {
        let id = nodes(ui.tree())[0];
        ui.tree().node_ref::<TableBody>(id).expect("TableBody")
    }

    /// How many cells actually became nodes.
    fn sel_di_pohon(ui: &AppRuntime) -> usize {
        fn hitung(tree: &silka_core::tree::RenderTree, id: silka_core::tree::NodeId) -> usize {
            usize::from(tree.node_ref::<TableCellBox>(id).is_some())
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
    fn seratus_ribu_baris_berkolom_hanya_menjadi_belasan_node() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        diam(&mut ui);

        let sel = sel_di_pohon(&ui);
        assert!(sel > 0, "tabel tidak membangun satu sel pun");
        assert!(
            sel < 4 * 40,
            "seratus ribu baris menjadi {sel} sel — virtualisasi bocor"
        );
        assert_eq!(body(&ui).metrics().count, BARIS);
        assert!(ui.is_idle(), "halaman diam tidak menyisakan pekerjaan");
    }

    #[test]
    fn menggulir_seratus_ribu_baris_tidak_membengkakkan_pohon() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        diam(&mut ui);
        let awal = sel_di_pohon(&ui);

        // Forty big jumps across the whole data set: the number of live nodes
        // must not grow by one. This is "scrolling must stay smooth" in a form
        // that can be tested without eyes.
        let mut maksimum = awal;
        for i in 1..=40 {
            let target = i * (BARIS / 40);
            ui.dispatch(&Event::Key(KeyEvent::pressed(
                KeyCode::Named(NamedKey::Tab),
                Duration::ZERO,
            )));
            body(&ui).state().unwrap().scroll_to_row(target, BARIS);
            diam(&mut ui);
            maksimum = maksimum.max(sel_di_pohon(&ui));
        }
        assert!(
            maksimum <= awal + 4 * 8,
            "jendela membengkak saat digulir ({awal} → {maksimum})"
        );
        assert!(body(&ui).first() > 90_000);
    }

    #[test]
    fn tabel_barisnya_dan_selnya_terbaca_screen_reader() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        diam(&mut ui);

        let pohon = ui.access_tree();
        let tabel = pohon
            .find_role(AccessRole::Table)
            .unwrap_or_else(|| panic!("{}", pohon.dump()));
        assert_eq!(tabel.node.label.as_deref(), Some(NAMA_TABEL));
        assert!(tabel.node.actions.contains(AccessActions::FOCUS));

        let baris = pohon
            .entries()
            .iter()
            .filter(|e| e.node.role == AccessRole::Row)
            .count();
        assert!(
            baris > 1,
            "tidak ada baris di pohon a11y:\n{}",
            pohon.dump()
        );
        let sel = pohon
            .entries()
            .iter()
            .filter(|e| e.node.role == AccessRole::Cell)
            .count();
        assert!(sel > 4, "tidak ada sel di pohon a11y:\n{}", pohon.dump());
        // The column headings are announced, and the first row really does
        // carry its data.
        assert!(pohon.find_label("Nominal").is_some());
        assert!(pohon.find_label("#000001").is_some());
    }

    #[test]
    fn klik_judul_kolom_mengurutkan_seratus_ribu_baris() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        diam(&mut ui);

        let sebelum = kotak(&ui, "#000001");
        assert!(sebelum.size.width > 0.0);

        let judul = kotak(&ui, "Pihak").center();
        klik(&mut ui, judul, 1, Duration::from_secs(1));
        assert_eq!(
            body(&ui).state().unwrap().sort(),
            Some(SortBy::ascending(1))
        );
        // The first row changes: the data really is sorted, not just the arrow
        // moving.
        let pohon = ui.access_tree();
        assert!(
            pohon.find_label("Apotek Sehat").is_some(),
            "kolom tidak terurut menaik:\n{}",
            pohon.dump()
        );

        // A second click reverses the direction — and still does not touch the
        // node count.
        klik(&mut ui, judul, 1, Duration::from_secs(4));
        assert_eq!(
            body(&ui).state().unwrap().sort(),
            Some(SortBy::descending(1))
        );
        assert!(sel_di_pohon(&ui) < 4 * 40);
    }

    #[test]
    fn seleksi_jamak_lewat_keyboard_dan_status_ikut_berubah() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        diam(&mut ui);

        // Tab until the table holds focus.
        for _ in 0..8 {
            tombol(&mut ui, NamedKey::Tab);
            if body(&ui).is_focused() {
                break;
            }
        }
        assert!(body(&ui).is_focused(), "tabel tidak bisa dicapai Tab");

        tombol(&mut ui, NamedKey::Home);
        for _ in 0..2 {
            ui.dispatch(&Event::Key(
                KeyEvent::pressed(KeyCode::Named(NamedKey::ArrowDown), Duration::ZERO)
                    .modifiers(Modifiers::SHIFT),
            ));
            diam(&mut ui);
        }
        assert_eq!(body(&ui).selection().len(), 3);
        let pohon = ui.access_tree();
        assert!(
            pohon.entries().iter().any(|e| e
                .node
                .label
                .as_deref()
                .is_some_and(|l| l.contains("3 baris"))),
            "status tidak melaporkan seleksi jamak:\n{}",
            pohon.dump()
        );

        // ← → move by cell, not by row.
        tombol(&mut ui, NamedKey::ArrowRight);
        assert_eq!(body(&ui).active_column(), 1);
        assert_eq!(body(&ui).selection().len(), 3, "seleksi ikut berubah");
    }

    #[test]
    fn tabel_kosong_menampilkan_empty_state_dan_tetap_berjudul() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        diam(&mut ui);

        let p = kotak(&ui, TOMBOL_KOSONG).center();
        klik(&mut ui, p, 1, Duration::from_secs(1));

        let pohon = ui.access_tree();
        assert!(
            pohon.find_label(KOSONG).is_some(),
            "empty state tidak muncul:\n{}",
            pohon.dump()
        );
        assert!(pohon.find_label("Nominal").is_some(), "judul kolom hilang");
        assert_eq!(body(&ui).metrics().count, 0);

        // And back again.
        let p = kotak(&ui, TOMBOL_ISI).center();
        klik(&mut ui, p, 1, Duration::from_secs(4));
        assert_eq!(body(&ui).metrics().count, BARIS);
    }

    #[test]
    fn tombol_lompat_jauh_menggulirkan_seratus_ribu_baris() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        diam(&mut ui);

        let p = kotak(&ui, TOMBOL_TENGAH).center();
        klik(&mut ui, p, 1, Duration::from_secs(1));
        assert!(
            body(&ui).first() >= 49_000,
            "jendela tidak ikut melompat: {}",
            body(&ui).first()
        );
        assert!(
            sel_di_pohon(&ui) < 4 * 40,
            "jendela membengkak setelah lompat"
        );
    }

    #[test]
    fn kolom_mengisi_lebar_dan_header_sejalan_dengan_barisnya() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        diam(&mut ui);

        let widths = body(&ui).column_widths();
        assert_eq!(widths.len(), 4);
        let id = header_nodes(ui.tree())[0];
        let header = ui.tree().node_ref::<TableHeaderBox>(id).unwrap();
        assert_eq!(
            header.column_widths(),
            widths,
            "judul kolom tidak sejajar dengan barisnya"
        );
    }

    #[test]
    fn benar_di_kedua_preset_dan_kedua_appearance() {
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let f = fonts();
                let mut ui = ui(t, &f);
                diam(&mut ui);
                assert_eq!(ui.scene().clear_color(), t.color.background);
                assert!(
                    sel_di_pohon(&ui) > 0,
                    "tabel kosong di {preset:?} {appearance:?}"
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
