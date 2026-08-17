//! Demo page: **virtualized tree** (`KOMPONEN.md` Tier 5, `NSOutlineView`).
//!
//! The number is as absurd as on the `list` and `table` pages, and for the same
//! reason: **fifty thousand files** in a thousand folders across fifty volumes.
//! A tree that is "fast" with two hundred nodes proves nothing; a tree that
//! opens, closes, scrolls, and answers the keyboard across fifty thousand nodes
//! proves the virtualization does not leak on any path — including the one that
//! is easiest to get wrong, the frames **during** a disclosure animation.
//!
//! | What it proves | How to try it in the window |
//! |---|---|
//! | Virtualization | "Buka semua", then scroll to file 40,000: no stutter, and only a dozen rows are ever nodes |
//! | No third virtualization system | Scrolling, rubber banding, and the scrollbar belong to `scroll_view`; the row window belongs to the same `ListMetrics` the `list` page uses |
//! | Height animation | Click a chevron: the subtree does not appear, it **grows**, and the rows below slide down on a spring |
//! | Rotating chevron | Watch the triangle while it opens — it turns, it does not swap pictures |
//! | Indentation + guides | Every level steps in, and the connector lines say which parent a row belongs to |
//! | Lazy loading | A folder's children are fetched **the moment it opens**, never before — the counter under the tree only moves when you open something |
//! | Selection | Click, ⇧-click to extend, ⌘-click to pick individually, ⌘A for everything |
//! | Full keyboard | Tab to the tree, then ↑ ↓ · → opens or steps in · ← closes or steps out · Home/End · Enter · **type a letter to jump** |
//! | Empty state | "Kosongkan" — an empty tree says so instead of showing a blank box |
//! | AccessKit nodes | VoiceOver says "tree", and each row announces its level, its position among its siblings, and whether it is open |
//! | Both presets & dark mode | `--preset tailwind`, `--appearance dark` |
//! | Reduced motion | Turn on "Reduce motion": the highlight is instantly in place, while the disclosure keeps moving — it is what explains where the rows came from |
//!
//! What is **absent** from this file: hand-assembled `Scene`s, layout
//! arithmetic, and color numbers. Everything is a token (§2.6, §2.7).

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use silka_core::app::{component, BuildCtx, ScaleFactor};
use silka_core::signals::{use_signal, Signal};
use silka_core::tree::{BoxConstraints, CrossAlign, MainAlign};
use silka_core::view::{column, constrained, row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{
    button_in, button_variant_in, spacer, text_in, tree_in, use_tree_state, ButtonVariant, Fonts,
    TreeKey, TreeNode, TreeRow, TreeState,
};

/// The page title.
pub const JUDUL: &str = "Tree (tervirtualisasi)";
/// The tree's name for screen readers — and the anchor the tests look for.
pub const NAMA_POHON: &str = "Berkas";

/// How many volumes (roots).
pub const VOLUME: u64 = 50;
/// How many folders per volume.
pub const FOLDER: u64 = 20;
/// How many files per folder.
pub const BERKAS: u64 = 50;
/// Total number of nodes when everything is open — the number the demo is
/// really about.
#[allow(dead_code)]
pub const TOTAL: usize = (VOLUME + VOLUME * FOLDER + VOLUME * FOLDER * BERKAS) as usize;

/// The expand-everything button.
pub const TOMBOL_SEMUA: &str = "Buka semua";
/// The collapse-everything button.
pub const TOMBOL_TUTUP: &str = "Tutup semua";
/// The jump-far button.
pub const TOMBOL_JAUH: &str = "Ke berkas 40.000";
/// The button that empties the tree (showing off the empty state).
pub const TOMBOL_KOSONG: &str = "Kosongkan";
/// The button that puts the content back.
pub const TOMBOL_ISI: &str = "Isi lagi";

/// The empty-state text.
pub const KOSONG: &str = "Tidak ada berkas";

/// One row's height — which is also the HIG's minimum hit target.
const TINGGI_BARIS: f32 = 44.0;
/// The tree viewport's height, in spacing-scale steps.
const TINGGI_LANGKAH: f32 = 92.0;
/// The tree's maximum width, in spacing-scale steps.
const LEBAR_LANGKAH: f32 = 150.0;

// ---------------------------------------------------------------------------
// A hierarchy nobody ever holds in memory
// ---------------------------------------------------------------------------
//
// Not one of the fifty thousand nodes is stored anywhere: keys encode the path,
// and a level is generated the moment it is asked for. That is the honest shape
// of a large tree — a file system, a database, an API — and it is also what
// makes the lazy-loading hook meaningful rather than decorative.

/// What a key is: a volume, a folder, or a file.
enum Simpul {
    Volume(u64),
    Folder(u64),
    Berkas(u64),
}

fn jenis(key: TreeKey) -> Simpul {
    if key < VOLUME {
        Simpul::Volume(key)
    } else if key < VOLUME + VOLUME * FOLDER {
        Simpul::Folder(key - VOLUME)
    } else {
        Simpul::Berkas(key - VOLUME - VOLUME * FOLDER)
    }
}

/// The key of folder `f` inside volume `v`.
fn kunci_folder(v: u64, f: u64) -> TreeKey {
    VOLUME + v * FOLDER + f
}

/// The key of file `b` inside the folder with **folder index** `f`.
fn kunci_berkas(f: u64, b: u64) -> TreeKey {
    VOLUME + VOLUME * FOLDER + f * BERKAS + b
}

/// Every folder key — what "Buka semua" opens.
fn semua_folder() -> Vec<TreeKey> {
    (0..VOLUME)
        .flat_map(|v| (0..FOLDER).map(move |f| kunci_folder(v, f)))
        .chain(0..VOLUME)
        .collect()
}

/// The display name of a node.
fn nama(key: TreeKey) -> String {
    match jenis(key) {
        Simpul::Volume(v) => format!("{} ({})", NAMA_VOLUME[(v % 5) as usize], v + 1),
        Simpul::Folder(f) => NAMA_FOLDER[(f % 8) as usize].to_string(),
        Simpul::Berkas(b) => format!("berkas-{:05}.txt", b + 1),
    }
}

const NAMA_VOLUME: [&str; 5] = ["Macintosh HD", "Arsip", "Cadangan", "Proyek", "Media"];

const NAMA_FOLDER: [&str; 8] = [
    "Dokumen", "Gambar", "Musik", "Unduhan", "Laporan", "Kontrak", "Faktur", "Rekaman",
];

/// The size shown next to a file — fake data that still looks like data.
fn ukuran(b: u64) -> String {
    let kb = (b * 8_191) % 9_000 + 12;
    if kb < 1_000 {
        format!("{kb} KB")
    } else {
        format!("{},{} MB", kb / 1_000, (kb % 1_000) / 100)
    }
}

/// What has been "loaded" so far, plus a version the tree can watch.
///
/// Behind a [`RefCell`] rather than a signal on purpose: it is derived from
/// what the user opened, and writing to it must not schedule a frame of its own
/// — the expansion change already did.
#[derive(Default)]
struct Muatan {
    dimuat: BTreeSet<TreeKey>,
    versi: u64,
}

impl Muatan {
    fn muat(&mut self, key: TreeKey) {
        if self.dimuat.insert(key) {
            self.versi = self.versi.wrapping_add(1);
        }
    }

    fn muat_semua(&mut self, keys: impl IntoIterator<Item = TreeKey>) {
        let sebelum = self.dimuat.len();
        self.dimuat.extend(keys);
        if self.dimuat.len() != sebelum {
            self.versi = self.versi.wrapping_add(1);
        }
    }

    fn sudah(&self, key: TreeKey) -> bool {
        self.dimuat.contains(&key)
    }
}

// ---------------------------------------------------------------------------
// The page
// ---------------------------------------------------------------------------

/// The view tree for the whole page — this is what gets handed to
/// `run_app_with`.
pub fn halaman(cx: &BuildCtx, fonts: &Fonts) -> View {
    let t: Theme = cx.expect_env::<Signal<Theme>>().get();
    // Text is rasterized at the real screen resolution (§3.3).
    let dpi: ScaleFactor = cx.expect_env::<Signal<ScaleFactor>>().get();
    fonts.set_scale_factor(dpi.get());

    let pohon_state = use_tree_state();
    let dibuka = use_signal(|| None::<TreeKey>);
    let terisi = use_signal(|| true);
    let muatan = use_signal(|| Rc::new(RefCell::new(Muatan::default())));
    // The lazy-loading counter is a signal of its own so that reading it
    // rebuilds only the status line, never the tree (§2.5).
    let dimuat = use_signal(|| 0usize);

    column([
        View::from(
            text_in(fonts, JUDUL)
                .size(t.typography.title2.size)
                .weight(FontWeight::SEMIBOLD)
                .tracking(t.typography.title2.tracking)
                .color(t.color.label)
                .single_line(),
        ),
        View::from(
            text_in(
                fonts,
                "Lima puluh ribu berkas dalam seribu folder, dan hanya belasan \
                 baris yang pernah menjadi node — memakai ulang virtualisasi \
                 yang sama dengan komponen list, bukan sistem ketiga. Klik \
                 segitiga untuk membuka: subpohonnya tidak muncul begitu saja, \
                 ia tumbuh, dan baris di bawahnya bergeser dengan pegas. Isi \
                 folder baru diambil saat folder itu dibuka.",
            )
            .size(t.typography.body_size)
            .line_height(t.typography.body_line_height)
            .color(t.color.secondary_label)
            .max_width(t.space(LEBAR_LANGKAH)),
        ),
        pohon(fonts, &t, pohon_state, dibuka, terisi, muatan, dimuat),
        kendali(fonts, &t, pohon_state, terisi, muatan, dimuat),
        status(fonts, pohon_state, dibuka, dimuat),
    ])
    .spacing(t.space(5.0))
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(8.0)))
    .into()
}

/// The tree viewport.
///
/// The scroll axis **must** be bounded (the same rule as Flutter's): the bound
/// lives here, not inside the container.
fn pohon(
    fonts: &Fonts,
    t: &Theme,
    state: TreeState,
    dibuka: Signal<Option<TreeKey>>,
    terisi: Signal<bool>,
    muatan: Signal<Rc<RefCell<Muatan>>>,
    dimuat: Signal<usize>,
) -> View {
    let ada = terisi.get();
    let simpanan = muatan.peek();
    let versi = simpanan.borrow().versi;

    // The source: called **only** for folders that are actually open, and it
    // answers nothing at all until that folder has been loaded.
    let untuk_sumber = simpanan.clone();
    let sumber = move |induk: Option<TreeKey>| -> Vec<TreeNode> {
        if !ada {
            return Vec::new();
        }
        let muatan = untuk_sumber.borrow();
        match induk {
            None => (0..VOLUME).map(|v| TreeNode::branch(v, nama(v))).collect(),
            Some(k) if !muatan.sudah(k) => Vec::new(),
            Some(k) => match jenis(k) {
                Simpul::Volume(v) => (0..FOLDER)
                    .map(|f| {
                        let key = kunci_folder(v, f);
                        TreeNode::branch(key, nama(key))
                    })
                    .collect(),
                Simpul::Folder(f) => (0..BERKAS)
                    .map(|b| {
                        let key = kunci_berkas(f, b);
                        TreeNode::leaf(key, nama(key))
                    })
                    .collect(),
                Simpul::Berkas(_) => Vec::new(),
            },
        }
    };

    let untuk_baris = fonts.clone();
    let untuk_kosong = fonts.clone();
    let theme = *t;
    let untuk_buka = simpanan.clone();

    constrained(
        BoxConstraints::new(
            0.0,
            t.space(LEBAR_LANGKAH),
            t.space(TINGGI_LANGKAH),
            t.space(TINGGI_LANGKAH),
        ),
        tree_in(t, state, sumber, move |r| baris(&untuk_baris, &theme, r))
            .row_extent(TINGGI_BARIS)
            .guides(t.space(0.25))
            .multi_selection()
            // Everything the source's answer depends on has to be in this
            // number, `terisi` included — the flattening is cached, and a
            // source that quietly started answering differently would never be
            // asked again.
            .data_version(versi.wrapping_mul(2) + u64::from(ada))
            .label(NAMA_POHON)
            .background(t.color.surface_sunken)
            .corners(t.corners(t.radius.lg))
            .border(t.space(0.25), t.color.separator)
            .empty(move || kosong(&untuk_kosong, &theme))
            // **The lazy-loading hook.** The children are fetched exactly when
            // the folder opens, and the version bump is what tells the tree to
            // ask the source again.
            .on_expand(move |key| {
                untuk_buka.borrow_mut().muat(key);
                dimuat.set(untuk_buka.borrow().dimuat.len());
            })
            .on_activate(move |key| dibuka.set(Some(key))),
    )
    .into()
}

/// One row: the name, and what the node has to say for itself.
///
/// Called **only** for rows that are actually visible — that is virtualization's
/// promise, and that is why fifty thousand files are allowed here.
fn baris(fonts: &Fonts, t: &Theme, r: &TreeRow) -> View {
    let judul = text_in(fonts, r.label.to_string())
        .size(t.typography.body_size)
        .weight(if r.expandable {
            FontWeight::MEDIUM
        } else {
            FontWeight::REGULAR
        })
        .color(t.color.label)
        .single_line();

    let keterangan = match jenis(r.key) {
        Simpul::Volume(_) => format!("{FOLDER} folder"),
        Simpul::Folder(_) => format!("{BERKAS} berkas"),
        Simpul::Berkas(b) => ukuran(b),
    };

    row([
        View::from(judul),
        // A spacer: the right-hand column is always trailing-aligned, without a
        // single layout number on this page.
        View::from(spacer()),
        View::from(
            text_in(fonts, keterangan)
                .size(t.typography.footnote.size)
                .color(t.color.tertiary_label)
                .single_line(),
        ),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .padding(Insets::symmetric(t.space(2.0), 0.0))
    .into()
}

/// What an empty tree shows instead of a blank box.
fn kosong(fonts: &Fonts, t: &Theme) -> View {
    column([View::from(
        text_in(fonts, KOSONG)
            .size(t.typography.body_size)
            .color(t.color.tertiary_label)
            .single_line(),
    )])
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(Insets::all(t.space(6.0)))
    .into()
}

/// The buttons — proof that opening everything, closing everything, and
/// jumping far all work on fifty thousand nodes.
fn kendali(
    fonts: &Fonts,
    t: &Theme,
    state: TreeState,
    terisi: Signal<bool>,
    muatan: Signal<Rc<RefCell<Muatan>>>,
    dimuat: Signal<usize>,
) -> View {
    let untuk_semua = muatan.peek();
    let ada = terisi.get();

    row([
        View::from(button_in(fonts, t, TOMBOL_SEMUA).on_press(move || {
            // "Expand all" loads first and then opens: a single rebuild, and —
            // deliberately — no height animation (§3.5, `open_many`).
            untuk_semua.borrow_mut().muat_semua(semua_folder());
            dimuat.set(untuk_semua.borrow().dimuat.len());
            state.open_many(semua_folder());
        })),
        View::from(
            button_variant_in(fonts, t, TOMBOL_TUTUP, ButtonVariant::Secondary).on_press(
                move || {
                    state.collapse_all();
                },
            ),
        ),
        View::from(
            button_variant_in(fonts, t, TOMBOL_JAUH, ButtonVariant::Secondary).on_press(
                move || {
                    let baris = state.flat().len();
                    state.scroll_to_row(baris.saturating_sub(11_000), baris);
                },
            ),
        ),
        View::from(
            button_variant_in(
                fonts,
                t,
                if ada { TOMBOL_KOSONG } else { TOMBOL_ISI },
                ButtonVariant::Ghost,
            )
            .on_press(move || terisi.set(!ada)),
        ),
    ])
    .spacing(t.space(3.0))
    .cross(CrossAlign::Center)
    .into()
}

/// The status line — **the only place the selection is read**, so moving the
/// highlight rebuilds just this text (§2.5).
fn status(
    fonts: &Fonts,
    state: TreeState,
    dibuka: Signal<Option<TreeKey>>,
    dimuat: Signal<usize>,
) -> View {
    let fonts = fonts.clone();
    component("status-pohon", move |cx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let seleksi = state.selection();
        let terlihat = state.flat().len();
        let terpilih = match seleksi.len() {
            0 => "belum ada".to_string(),
            1 => seleksi
                .lead()
                .and_then(|i| state.flat().get(i).map(|r| r.label.to_string()))
                .unwrap_or_else(|| "satu baris".to_string()),
            n => format!("{n} baris"),
        };
        let aktif = dibuka
            .get()
            .map(|k| format!("dibuka {}", nama(k)))
            .unwrap_or_else(|| "ketuk-ganda atau Enter untuk membuka".to_string());
        text_in(
            &fonts,
            format!(
                "Baris terlihat: {terlihat} · folder dimuat: {} · terpilih: {terpilih} · {aktif}",
                dimuat.get()
            ),
        )
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
    use silka_widgets::tree::{nodes, TreeBody, TreeRowBox};
    use std::time::Duration;

    const VIEWPORT: Size = Size::new(1000.0, 820.0);

    fn fonts() -> Fonts {
        Fonts::bundled_only()
    }

    /// A headless app assembled **exactly the way `run_app_with` does it**.
    fn ui(theme: Theme, fonts: &Fonts) -> AppRuntime {
        let untuk_view = fonts.clone();
        headless_app(theme, move |cx| halaman(cx, &untuk_view))
            .sized(VIEWPORT.width, VIEWPORT.height)
    }

    /// Pump frames until the app is genuinely at rest **and** no spring is
    /// still pending.
    fn diam(ui: &mut AppRuntime) {
        for _ in 0..16 {
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

    fn pohon_node(ui: &AppRuntime) -> &TreeBody {
        let id = nodes(ui.tree())[0];
        ui.tree().node_ref::<TreeBody>(id).expect("TreeBody")
    }

    /// How many rows actually became nodes.
    fn baris_di_pohon(ui: &AppRuntime) -> usize {
        fn hitung(tree: &silka_core::tree::RenderTree, id: silka_core::tree::NodeId) -> usize {
            usize::from(tree.node_ref::<TreeRowBox>(id).is_some())
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

    fn klik(ui: &mut AppRuntime, titik: Point, mulai: Duration) {
        ui.dispatch(&Event::Pointer(PointerEvent::new(
            PointerPhase::Move,
            titik,
            mulai,
        )));
        ui.dispatch(&Event::Pointer(
            PointerEvent::new(PointerPhase::Down, titik, mulai).button(PointerButton::Primary),
        ));
        ui.dispatch(&Event::Pointer(
            PointerEvent::new(PointerPhase::Up, titik, mulai + Duration::from_millis(10))
                .button(PointerButton::Primary),
        ));
        diam(ui);
    }

    /// True when the status line contains `teks`.
    fn memuat(ui: &AppRuntime, teks: &str) -> bool {
        ui.access_tree()
            .entries()
            .iter()
            .any(|e| e.node.label.as_deref().is_some_and(|l| l.contains(teks)))
    }

    fn tombol(ui: &mut AppRuntime, key: NamedKey) {
        ui.dispatch(&Event::Key(KeyEvent::pressed(
            KeyCode::Named(key),
            Duration::ZERO,
        )));
        diam(ui);
    }

    #[test]
    fn halaman_terbuka_dengan_volumenya_saja() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        diam(&mut ui);

        assert_eq!(
            pohon_node(&ui).flat().len(),
            VOLUME as usize,
            "pohon tertutup harus hanya menampilkan volumenya"
        );
        assert!(baris_di_pohon(&ui) > 0, "tidak ada baris sama sekali");
        assert!(ui.is_idle(), "halaman diam menyisakan pekerjaan");
    }

    #[test]
    fn buka_semua_lima_puluh_ribu_simpul_tetap_belasan_node() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        diam(&mut ui);

        let p = kotak(&ui, TOMBOL_SEMUA).center();
        klik(&mut ui, p, Duration::from_secs(1));

        assert_eq!(
            pohon_node(&ui).flat().len(),
            TOTAL,
            "seluruh pohon harus terbuka"
        );
        let baris = baris_di_pohon(&ui);
        assert!(
            baris < 60,
            "lima puluh ribu simpul menjadi {baris} node — virtualisasi bocor"
        );

        // And it stays that way after jumping deep into the data.
        let p = kotak(&ui, TOMBOL_JAUH).center();
        klik(&mut ui, p, Duration::from_secs(4));
        assert!(
            pohon_node(&ui).first() > 30_000,
            "jendela tidak ikut melompat: {}",
            pohon_node(&ui).first()
        );
        assert!(
            baris_di_pohon(&ui) < 60,
            "jendela membengkak setelah melompat"
        );
    }

    #[test]
    fn folder_dimuat_saat_dibuka_bukan_sebelumnya() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        diam(&mut ui);
        assert!(
            memuat(&ui, "folder dimuat: 0"),
            "ada yang sudah dimuat padahal belum ada yang dibuka:\n{}",
            ui.access_tree().dump()
        );

        // Open the first volume by keyboard — the same path a screen-reader
        // user takes.
        for _ in 0..8 {
            tombol(&mut ui, NamedKey::Tab);
            if pohon_node(&ui).is_focused() {
                break;
            }
        }
        assert!(pohon_node(&ui).is_focused(), "pohon tidak bisa dicapai Tab");
        tombol(&mut ui, NamedKey::ArrowRight);

        assert_eq!(
            pohon_node(&ui).flat().len(),
            (VOLUME + FOLDER) as usize,
            "isi folder tidak muncul setelah dibuka"
        );
        assert!(
            memuat(&ui, "folder dimuat: 1"),
            "penghitung pemuatan tidak ikut bergerak:\n{}",
            ui.access_tree().dump()
        );
    }

    #[test]
    fn pohon_dan_barisnya_terbaca_screen_reader() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Light), &f);
        diam(&mut ui);

        let pohon = ui.access_tree();
        let akar = pohon
            .find_role(AccessRole::Tree)
            .unwrap_or_else(|| panic!("{}", pohon.dump()));
        assert_eq!(akar.node.label.as_deref(), Some(NAMA_POHON));
        assert!(akar.node.actions.contains(AccessActions::FOCUS));

        let baris: Vec<_> = pohon
            .entries()
            .iter()
            .filter(|e| e.node.role == AccessRole::TreeItem)
            .collect();
        assert!(!baris.is_empty(), "tidak ada baris di pohon a11y");
        let pertama = &baris[0].node;
        assert_eq!(pertama.level, Some(1));
        assert_eq!(pertama.position_in_set, Some(1));
        assert_eq!(pertama.size_of_set, Some(VOLUME as usize));
        assert_eq!(pertama.expanded, Some(false), "volume tertutup");
        assert!(pertama.actions.contains(AccessActions::EXPAND));
    }

    #[test]
    fn kosongkan_menampilkan_empty_state() {
        let f = fonts();
        let mut ui = ui(Theme::cupertino(Appearance::Dark), &f);
        diam(&mut ui);

        let p = kotak(&ui, TOMBOL_KOSONG).center();
        klik(&mut ui, p, Duration::from_secs(1));
        assert_eq!(pohon_node(&ui).flat().len(), 0);
        assert_eq!(baris_di_pohon(&ui), 0);
        assert!(
            ui.access_tree().find_label(KOSONG).is_some(),
            "empty state tidak terbaca:\n{}",
            ui.access_tree().dump()
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
                    baris_di_pohon(&ui) > 0,
                    "pohon kosong di {preset:?} {appearance:?}"
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
