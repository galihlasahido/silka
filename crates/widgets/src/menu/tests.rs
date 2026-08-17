//! `menu` tests — no GPU, no system fonts, no window (§9.5).
//!
//! Two layers, deliberately kept apart:
//!
//! 1. **The rules** — [`MenuState::apply`] and the navigation helpers are pure
//!    functions, so every keyboard rule is checked as arithmetic.
//! 2. **The assembled component** — a real render tree, real layout, real input
//!    routing, and the real a11y tree, driven through the same frame cycle an
//!    application uses (`advance` → rebuild). That second layer is what catches
//!    the class of bug the first cannot: a rule that is right on paper and
//!    never reaches the screen (the lesson recorded in `catatan/STATUS.md`).

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use silka_core::access::{AccessActions, AccessRole, AccessToggled};
use silka_core::animation::{Motion, Tick};
use silka_core::input::{
    Event, InputRouter, KeyCode, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent,
    PointerPhase,
};
use silka_core::signals::{Runtime, Signal};
use silka_core::tree::{BoxConstraints, NodeId, RenderTree};
use silka_core::view::{column, reconcile, View};
use silka_paint::{Point, Rect, Size};
use silka_theme::{Appearance, Preset, Theme};

use super::*;
use crate::overlay::{entries as overlay_entries, overlay_layer, OverlayEntry};
use crate::Fonts;

const LAYAR: Size = Size::new(720.0, 520.0);
const SEFRAME: Duration = Duration::from_millis(8);

fn tema() -> Theme {
    Theme::cupertino(Appearance::Dark)
}

/// The menu every test uses: two commands with shortcuts, a separator, a
/// checkable pair, a disabled row, and a submenu.
fn isi() -> Vec<MenuEntry> {
    vec![
        item("zoom.in", "Perbesar")
            .shortcut(cmd(KeyCode::Character('+')))
            .into(),
        item("zoom.out", "Perkecil")
            .shortcut(cmd(KeyCode::Character('-')))
            .into(),
        separator(),
        item("view.grid", "Tampilkan kisi").checkbox(true).into(),
        item("view.ruler", "Tampilkan penggaris")
            .checkbox(false)
            .into(),
        item("view.export", "Ekspor…").enabled(false).into(),
        item("view.sort", "Urutkan")
            .submenu([
                item("sort.name", "Nama").radio(true),
                item("sort.date", "Tanggal").radio(false),
            ])
            .into(),
    ]
}

/// Index of each interesting row of the root level, named so the tests read
/// like sentences instead of like arithmetic.
const PERBESAR: usize = 0;
const PERKECIL: usize = 1;
const PEMISAH: usize = 2;
const KISI: usize = 3;
const PENGGARIS: usize = 4;
const EKSPOR: usize = 5;
const URUTKAN: usize = 6;

fn model() -> MenuModel {
    MenuModel::new(isi())
}

// ---------------------------------------------------------------------------
// The rules — pure functions
// ---------------------------------------------------------------------------

#[test]
fn pintasan_ditulis_menurut_konvensi_tiap_os() {
    let s = shortcut(
        Modifiers::META.union(Modifiers::SHIFT),
        KeyCode::Character('s'),
    );
    // macOS: symbols, fixed order ⌃⌥⇧⌘, uppercase key, no separator.
    assert_eq!(s.display(ShortcutStyle::Symbols), "⇧⌘S");
    // Windows/Linux: words joined by `+`.
    assert_eq!(s.display(ShortcutStyle::Words), "Shift+Meta+S");

    let semua = shortcut(
        Modifiers::CONTROL
            .union(Modifiers::ALT)
            .union(Modifiers::SHIFT)
            .union(Modifiers::META),
        KeyCode::Named(NamedKey::Enter),
    );
    assert_eq!(semua.display(ShortcutStyle::Symbols), "⌃⌥⇧⌘↩");
    assert_eq!(
        semua.display(ShortcutStyle::Words),
        "Ctrl+Alt+Shift+Meta+Enter"
    );

    // A key with no spelling prints nothing rather than a placeholder.
    let aneh = shortcut(Modifiers::NONE, KeyCode::Unidentified);
    assert_eq!(aneh.display(ShortcutStyle::Words), "");
}

#[test]
fn langkah_melompati_pemisah_dan_item_mati() {
    let entries = isi();
    // Down from the second command jumps **over** the separator.
    assert_eq!(step(&entries, Some(PERKECIL), 1), Some(KISI));
    // Down from the last checkable jumps over the disabled export row.
    assert_eq!(step(&entries, Some(PENGGARIS), 1), Some(URUTKAN));
    // And back up again, still skipping it.
    assert_eq!(step(&entries, Some(URUTKAN), -1), Some(PENGGARIS));
    // Nothing highlighted: ↓ starts at the top, ↑ at the bottom.
    assert_eq!(step(&entries, None, 1), Some(PERBESAR));
    assert_eq!(step(&entries, None, -1), Some(URUTKAN));
    // The ends wrap, the way every native menu does.
    assert_eq!(step(&entries, Some(URUTKAN), 1), Some(PERBESAR));
    assert_eq!(step(&entries, Some(PERBESAR), -1), Some(URUTKAN));
    // A separator is never a landing place.
    assert_ne!(step(&entries, Some(PERKECIL), 1), Some(PEMISAH));

    // A level with nothing selectable settles on nothing instead of spinning.
    let mati = vec![separator(), item("x", "X").enabled(false).into()];
    assert_eq!(step(&mati, None, 1), None);
    assert_eq!(step(&mati, Some(0), -1), None);
}

#[test]
fn mengetik_huruf_berputar_dari_posisi_sekarang() {
    let entries = isi();
    // "t" matches both "Tampilkan …" rows; typing it twice walks through them.
    assert_eq!(typeahead(&entries, "t", None), Some(KISI));
    assert_eq!(typeahead(&entries, "t", Some(KISI)), Some(PENGGARIS));
    assert_eq!(typeahead(&entries, "t", Some(PENGGARIS)), Some(KISI));
    // A longer prefix refines what is highlighted instead of moving on.
    assert_eq!(
        typeahead(&entries, "tampilkan p", Some(KISI)),
        Some(PENGGARIS)
    );
    // Case is irrelevant; a disabled row never matches.
    assert_eq!(typeahead(&entries, "PERB", None), Some(PERBESAR));
    assert_eq!(typeahead(&entries, "ekspor", None), None);
    assert_eq!(typeahead(&entries, "", None), None);
}

#[test]
fn jalur_menelusuri_submenu_dan_menolak_yang_tidak_ada() {
    let m = model();
    assert_eq!(m.level(&[]).unwrap().len(), isi().len());
    assert_eq!(m.level(&[URUTKAN]).unwrap().len(), 2);
    assert_eq!(m.item_at(&[URUTKAN], 1).unwrap().id(), "sort.date");
    // A path through an item without a submenu does not exist…
    assert!(m.level(&[PERBESAR]).is_none());
    // …and neither does one through a separator.
    assert!(m.level(&[PEMISAH]).is_none());
    assert_eq!(m.depth(), 2);
}

#[test]
fn membuka_tidak_menyorot_apa_pun() {
    let m = model();
    let mut s = MenuState::new();
    let jangkar = Anchor::Rect(Rect::new(10.0, 20.0, 120.0, 44.0));
    assert!(s.apply(MenuIntent::Open(jangkar), &m));
    assert!(s.is_open());
    assert_eq!(s.anchor, jangkar);
    assert_eq!(
        s.highlight, None,
        "menu yang langsung menyorot mengundang Return tak sengaja"
    );
    assert_eq!(s.visible_levels(), 1);
}

#[test]
fn esc_menutup_satu_tingkat_bukan_seluruhnya() {
    let m = model();
    let mut s = MenuState::new();
    s.apply(MenuIntent::Open(Anchor::None), &m);
    s.apply(
        MenuIntent::Highlight {
            depth: 0,
            index: Some(URUTKAN),
        },
        &m,
    );
    s.apply(MenuIntent::Descend, &m);
    assert_eq!(s.depth(), 1);
    assert_eq!(
        s.highlight,
        Some(0),
        "keyboard masuk ke baris pertama submenu"
    );

    // One level out: the parent row is highlighted again, the menu stays open.
    assert!(s.apply(MenuIntent::CloseLevel, &m));
    assert_eq!(s.depth(), 0);
    assert_eq!(s.highlight, Some(URUTKAN));
    assert!(s.is_open());

    // Once more and the menu itself closes.
    assert!(s.apply(MenuIntent::CloseLevel, &m));
    assert!(!s.is_open());
}

#[test]
fn menyorot_di_tingkat_atas_menutup_submenu_yang_terbuka() {
    let m = model();
    let mut s = MenuState::new();
    s.apply(MenuIntent::Open(Anchor::None), &m);
    s.apply(
        MenuIntent::OpenSubmenu {
            depth: 0,
            index: URUTKAN,
            anchor: Some(Anchor::Rect(Rect::new(0.0, 0.0, 100.0, 44.0))),
            focus_first: false,
        },
        &m,
    );
    assert_eq!(s.depth(), 1);
    // The pointer moves to another row of the parent level.
    s.apply(
        MenuIntent::Highlight {
            depth: 0,
            index: Some(KISI),
        },
        &m,
    );
    assert_eq!(s.depth(), 0, "submenu ikut tertutup");
    assert_eq!(s.highlight, Some(KISI));
}

#[test]
fn memilih_induk_submenu_membukanya_bukan_menjalankannya() {
    let m = model();
    let mut s = MenuState::new();
    s.apply(MenuIntent::Open(Anchor::None), &m);
    assert!(s.apply(
        MenuIntent::Activate {
            depth: 0,
            index: URUTKAN
        },
        &m
    ));
    assert!(s.is_open(), "menu tidak boleh tertutup");
    assert_eq!(s.depth(), 1);
    // The handler must not report an activation for it either.
    assert!(s.activated(&m, 0, URUTKAN).is_none());
    assert_eq!(
        s.activated(&m, 0, PERBESAR).map(MenuItem::id),
        Some("zoom.in")
    );
}

#[test]
fn item_mati_tidak_bisa_disorot_maupun_dijalankan() {
    let m = model();
    let mut s = MenuState::new();
    s.apply(MenuIntent::Open(Anchor::None), &m);
    assert!(!s.apply(
        MenuIntent::Activate {
            depth: 0,
            index: EKSPOR
        },
        &m
    ));
    assert!(s.is_open());
    s.apply(
        MenuIntent::Highlight {
            depth: 0,
            index: Some(EKSPOR),
        },
        &m,
    );
    assert_eq!(s.highlight, None, "baris mati tidak menerima sorotan");
    assert!(s.activated(&m, 0, EKSPOR).is_none());
    // A separator is just as unselectable.
    s.apply(
        MenuIntent::Highlight {
            depth: 0,
            index: Some(PEMISAH),
        },
        &m,
    );
    assert_eq!(s.highlight, None);
}

#[test]
fn menjalankan_item_menutup_seluruh_menu() {
    let m = model();
    let mut s = MenuState::new();
    s.apply(MenuIntent::Open(Anchor::None), &m);
    s.apply(MenuIntent::Descend, &m); // nothing highlighted: no-op
    s.apply(
        MenuIntent::Highlight {
            depth: 0,
            index: Some(URUTKAN),
        },
        &m,
    );
    s.apply(MenuIntent::Descend, &m);
    assert!(s.apply(MenuIntent::Activate { depth: 1, index: 1 }, &m));
    assert!(!s.is_open());
    assert_eq!(s.depth(), 0, "seluruh rantai submenu ikut tertutup");
}

#[test]
fn tingkat_tanpa_jangkar_belum_boleh_tampil() {
    let m = model();
    let mut s = MenuState::new();
    s.apply(MenuIntent::Open(Anchor::None), &m);
    s.apply(
        MenuIntent::Highlight {
            depth: 0,
            index: Some(URUTKAN),
        },
        &m,
    );
    s.apply(MenuIntent::Descend, &m);
    assert_eq!(s.depth(), 1);
    assert_eq!(
        s.visible_levels(),
        1,
        "panel tanpa jangkar tidak boleh digambar di tempat tebakan"
    );

    let kotak = Rect::new(40.0, 80.0, 160.0, 44.0);
    assert!(s.apply(
        MenuIntent::SubmenuAnchor {
            depth: 0,
            anchor: Anchor::Rect(kotak)
        },
        &m
    ));
    assert_eq!(s.visible_levels(), 2);
    // A second measurement of the same level changes nothing — no repaint loop.
    assert!(!s.apply(
        MenuIntent::SubmenuAnchor {
            depth: 0,
            anchor: Anchor::Rect(kotak)
        },
        &m
    ));
}

#[test]
fn sorotan_tingkat_atas_tetap_menandai_jalur_yang_terbuka() {
    let m = model();
    let mut s = MenuState::new();
    s.apply(MenuIntent::Open(Anchor::None), &m);
    s.apply(
        MenuIntent::Highlight {
            depth: 0,
            index: Some(URUTKAN),
        },
        &m,
    );
    s.apply(MenuIntent::Descend, &m);
    assert_eq!(s.highlight_at(0), Some(URUTKAN), "induk tetap tersorot");
    assert_eq!(s.highlight_at(1), Some(0));
    assert!(s.is_submenu_open(0, URUTKAN));
    assert!(!s.is_submenu_open(0, KISI));
}

#[test]
fn segitiga_submenu_mengarah_ke_akhir_baris() {
    let kotak = Rect::new(10.0, 4.0, 10.0, 16.0);
    let ltr = triangle_columns(kotak, false);
    let rtl = triangle_columns(kotak, true);
    assert_eq!(ltr.len(), rtl.len());
    // LTR: the base is at the left and the columns get shorter towards the tip.
    assert!(ltr[0].size.height > ltr[ltr.len() - 1].size.height);
    assert_eq!(ltr[0].min_x(), kotak.min_x());
    // RTL: mirrored — the base sits at the right edge.
    assert_eq!(rtl[0].max_x(), kotak.max_x());
    for k in ltr.iter().chain(rtl.iter()) {
        assert!(kotak.min_y() - 0.01 <= k.min_y() && k.max_y() <= kotak.max_y() + 0.01);
    }
    assert!(triangle_columns(Rect::new(0.0, 0.0, 0.0, 0.0), false).is_empty());
}

// ---------------------------------------------------------------------------
// The assembled component — a real tree, real input, real a11y
// ---------------------------------------------------------------------------

/// A whole page driven through the frame cycle of a real application.
struct Layar {
    _rt: Runtime,
    fonts: Fonts,
    theme: Theme,
    state: Signal<MenuState>,
    dipilih: Rc<RefCell<Vec<String>>>,
    tree: RenderTree,
    router: InputRouter,
    jam: Duration,
    kotak_pemicu: Rect,
    context: bool,
    /// The OS motion preference this page is driven under.
    gerak: Motion,
}

impl Layar {
    fn baru(theme: Theme) -> Self {
        Self::dengan(theme, Rect::new(40.0, 60.0, 0.0, 0.0), false)
    }

    /// `kotak` places the trigger; only its origin matters (the control sizes
    /// itself), and it is what lets a test put the trigger near the bottom edge
    /// to watch the panel flip.
    fn dengan(theme: Theme, kotak: Rect, context: bool) -> Self {
        let rt = Runtime::new();
        let state = rt.signal(MenuState::new());
        let mut layar = Self {
            _rt: rt,
            fonts: Fonts::bundled_only(),
            theme,
            state,
            dipilih: Rc::new(RefCell::new(Vec::new())),
            tree: RenderTree::new(),
            router: InputRouter::new(),
            jam: Duration::ZERO,
            kotak_pemicu: kotak,
            context,
            gerak: Motion::Full,
        };
        layar.bangun();
        layar
    }

    fn menu(&self) -> Menu {
        let dipilih = self.dipilih.clone();
        menu_in(&self.fonts, &self.theme, isi())
            .label("Tampilan")
            .key("uji")
            .bind(self.state)
            .on_activate(move |id| dipilih.borrow_mut().push(id.to_string()))
    }

    /// The page: the trigger inside the content, every panel in the layer.
    fn tampilan(&self) -> View {
        let m = self.menu();
        let pemicu = if self.context {
            // A canvas-sized region, so a right-click anywhere inside it is a
            // right-click on the region and not merely on its label.
            m.context_area(
                silka_core::view::fixed(320.0, 180.0).background(self.theme.color.surface),
            )
        } else {
            m.trigger("Tampilan")
        };
        // A padded column places the trigger where the test asked for it; the
        // overlay layer hands its content tight constraints, so a control
        // mounted straight into it would be stretched to the whole page.
        let konten = column([pemicu]).padding(silka_paint::Insets {
            left: self.kotak_pemicu.min_x(),
            top: self.kotak_pemicu.min_y(),
            right: 0.0,
            bottom: 0.0,
        });
        let mut layer = overlay_layer(konten);
        for panel in m.overlays() {
            layer = layer.overlay(panel);
        }
        layer.into()
    }

    fn bangun(&mut self) {
        let view = self.tampilan();
        reconcile(&mut self.tree, view);
        self.tree.layout(BoxConstraints::tight(LAYAR));
    }

    /// One frame, in the order the shell uses: animate, then rebuild.
    fn frame(&mut self) {
        self.jam += SEFRAME;
        let tick = Tick::manual(SEFRAME, self.gerak);
        crate::motion::advance(&mut self.tree, &tick);
        self.bangun();
    }

    /// Run frames until nothing moves **and** nothing is still waiting to be
    /// measured — with a cap, so work that never finishes becomes a failure
    /// instead of a hang.
    ///
    /// The second condition is the whole point of the sync pass: a row that
    /// still wants its rect means the application has a frame of work left,
    /// exactly as `AppRuntime::is_idle` would report.
    fn diamkan(&mut self) {
        for _ in 0..400 {
            self.frame();
            if !crate::motion::is_animating(&self.tree) && !self.menunggu_ukuran() {
                crate::motion::settle(&mut self.tree);
                self.tree.flush_layout();
                return;
            }
        }
        panic!("ada yang tidak pernah berhenti bergerak");
    }

    /// True while some row still owes the state its rect.
    fn menunggu_ukuran(&self) -> bool {
        self.cari::<MenuRowBox>().into_iter().any(|id| {
            self.tree
                .node_ref::<MenuRowBox>(id)
                .is_some_and(MenuRowBox::wants_anchor)
        })
    }

    fn cari<T: silka_core::tree::RenderNode>(&self) -> Vec<NodeId> {
        fn telusuri<T: silka_core::tree::RenderNode>(
            tree: &RenderTree,
            id: NodeId,
            out: &mut Vec<NodeId>,
        ) {
            if tree.node_ref::<T>(id).is_some() {
                out.push(id);
            }
            for anak in tree.children(id) {
                telusuri::<T>(tree, *anak, out);
            }
        }
        let mut out = Vec::new();
        telusuri::<T>(&self.tree, self.tree.root(), &mut out);
        out
    }

    fn pemicu(&self) -> NodeId {
        *self
            .cari::<MenuTriggerBox>()
            .first()
            .expect("pemicu ada di pohon")
    }

    fn kotak(&self, id: NodeId) -> Rect {
        self.tree.bounds(id)
    }

    /// Every row currently in the tree, in paint order.
    fn baris(&self) -> Vec<NodeId> {
        self.cari::<MenuRowBox>()
    }

    /// The rows of one level.
    fn baris_tingkat(&self, depth: usize) -> Vec<NodeId> {
        self.baris()
            .into_iter()
            .filter(|id| {
                self.tree
                    .node_ref::<MenuRowBox>(*id)
                    .is_some_and(|r| r.depth() == depth)
            })
            .collect()
    }

    /// The row at `(depth, index)`, looked up by the index it carries — not by
    /// its position among the rows, which separators shift.
    fn baris_ke(&self, depth: usize, index: usize) -> NodeId {
        self.baris()
            .into_iter()
            .find(|id| {
                self.tree
                    .node_ref::<MenuRowBox>(*id)
                    .is_some_and(|r| r.depth() == depth && r.index() == index)
            })
            .unwrap_or_else(|| panic!("tidak ada baris ({depth}, {index})"))
    }

    fn panel_terlihat(&self) -> Vec<Rect> {
        overlay_entries(&self.tree)
            .into_iter()
            .filter_map(|id| {
                let e = self.tree.node_ref::<OverlayEntry>(id)?;
                e.is_visible().then(|| {
                    let asal = self.tree.global_offset(id);
                    let r = e.panel_rect();
                    Rect::from_origin_size(
                        Point::new(r.min_x() + asal.x, r.min_y() + asal.y),
                        r.size,
                    )
                })
            })
            .collect()
    }

    fn a11y(&self) -> silka_core::access::AccessTree {
        self.tree.access_tree(self.router.focus().focused())
    }

    fn peran(&self, role: AccessRole) -> usize {
        self.a11y()
            .entries()
            .iter()
            .filter(|e| e.node.role == role)
            .count()
    }

    fn klik_di(&mut self, titik: Point, tombol: PointerButton) {
        for e in [
            PointerEvent::new(PointerPhase::Move, titik, self.jam),
            PointerEvent::new(PointerPhase::Down, titik, self.jam + SEFRAME).button(tombol),
            PointerEvent::new(PointerPhase::Up, titik, self.jam + SEFRAME * 2).button(tombol),
        ] {
            self.router.dispatch(&mut self.tree, &Event::Pointer(e));
        }
        self.diamkan();
    }

    fn arahkan(&mut self, titik: Point) {
        self.router.dispatch(
            &mut self.tree,
            &Event::Pointer(PointerEvent::new(PointerPhase::Move, titik, self.jam)),
        );
        self.diamkan();
    }

    fn tekan(&mut self, code: KeyCode) {
        self.router.dispatch(
            &mut self.tree,
            &Event::Key(KeyEvent::pressed(code, self.jam)),
        );
        self.diamkan();
    }

    fn ketik(&mut self, c: char) {
        self.tekan(KeyCode::Character(c));
    }

    fn buka_dengan_klik(&mut self) {
        let titik = self.kotak(self.pemicu()).center();
        self.klik_di(titik, PointerButton::Primary);
    }

    fn keadaan(&self) -> MenuState {
        self.state.peek()
    }
}

#[test]
fn klik_pemicu_membuka_panel_yang_menempel_di_bawahnya() {
    let mut layar = Layar::baru(tema());
    // A closed menu keeps its panel in the tree (that is what lets its
    // disappearance be animated), but it exists for nobody: no menu item is
    // announced, and no panel contributes pixels.
    assert_eq!(layar.peran(AccessRole::MenuItem), 0);
    assert_eq!(layar.peran(AccessRole::Menu), 0);
    assert!(layar.panel_terlihat().is_empty());

    let pemicu = layar.kotak(layar.pemicu());
    layar.buka_dengan_klik();

    assert!(layar.keadaan().is_open());
    assert_eq!(
        layar.baris_tingkat(0).len(),
        isi().len() - 1,
        "tanpa pemisah"
    );
    let panel = layar.panel_terlihat();
    assert_eq!(panel.len(), 1);
    // Placed by the overlay system, not by this component: below the trigger
    // and aligned to the start of the line.
    assert!(
        panel[0].min_y() >= pemicu.max_y(),
        "panel {:?} harus di bawah pemicu {:?}",
        panel[0],
        pemicu
    );
    assert!((panel[0].min_x() - pemicu.min_x()).abs() < 1.0);
    // …and it does not leave the screen.
    assert!(panel[0].max_x() <= LAYAR.width + 0.01);
    assert!(panel[0].max_y() <= LAYAR.height + 0.01);
}

#[test]
fn panel_membalik_ke_atas_di_tepi_bawah_layar() {
    // The trigger sits near the bottom edge: "below" no longer fits.
    let mut layar = Layar::dengan(
        tema(),
        Rect::new(40.0, LAYAR.height - 60.0, 0.0, 0.0),
        false,
    );
    let pemicu = layar.kotak(layar.pemicu());
    layar.buka_dengan_klik();

    let panel = layar.panel_terlihat();
    assert_eq!(panel.len(), 1);
    assert!(
        panel[0].max_y() <= pemicu.min_y() + 0.01,
        "panel {:?} harus membalik ke atas pemicu {:?} — geometrinya milik sistem overlay",
        panel[0],
        pemicu
    );
}

#[test]
fn baris_diumumkan_lengkap_dengan_peran_status_dan_hit_target() {
    let mut layar = Layar::baru(tema());
    layar.buka_dengan_klik();

    let pohon = layar.a11y();
    // The panel itself is a menu, and the separator is announced as one.
    assert!(layar.peran(AccessRole::Menu) >= 1, "panel berperan Menu");
    assert_eq!(layar.peran(AccessRole::Separator), 1);
    assert_eq!(layar.peran(AccessRole::MenuItem), isi().len() - 1);

    let baris = |label: &str| {
        pohon
            .find_label(label)
            .unwrap_or_else(|| panic!("tidak ada baris {label:?}:\n{}", pohon.dump()))
    };

    // Every row honours the HIG hit target, small as a menu row looks.
    for e in pohon.entries() {
        if e.node.role == AccessRole::MenuItem {
            assert!(
                e.bounds.size.height >= crate::MIN_HIT_TARGET,
                "baris {:?} cuma {:?}",
                e.node.label,
                e.bounds.size
            );
        }
    }

    // Checkable rows carry their state; a plain command carries none, so a
    // screen reader never announces "not checked" for a command.
    assert_eq!(
        baris("Tampilkan kisi").node.toggled,
        Some(AccessToggled::On)
    );
    assert_eq!(
        baris("Tampilkan penggaris").node.toggled,
        Some(AccessToggled::Off)
    );
    assert_eq!(baris("Perbesar").node.toggled, None);

    // The disabled row is still announced, dimmed and unclickable.
    let mati = baris("Ekspor…");
    assert!(mati.node.disabled);
    assert!(!mati.node.actions.contains(AccessActions::CLICK));

    // The submenu parent advertises that it opens something.
    let induk = baris("Urutkan");
    assert!(induk.node.actions.contains(AccessActions::EXPAND));
}

#[test]
fn klik_baris_menjalankan_lalu_menutup() {
    let mut layar = Layar::baru(tema());
    layar.buka_dengan_klik();

    let kotak = layar.kotak(layar.baris_ke(0, PERBESAR));
    layar.klik_di(kotak.center(), PointerButton::Primary);

    assert_eq!(layar.dipilih.borrow().as_slice(), ["zoom.in"]);
    assert!(!layar.keadaan().is_open(), "memilih selalu menutup menu");
    assert_eq!(layar.peran(AccessRole::MenuItem), 0);
}

#[test]
fn klik_di_luar_menutup_tanpa_menjalankan_apa_pun() {
    let mut layar = Layar::baru(tema());
    layar.buka_dengan_klik();
    assert!(layar.keadaan().is_open());

    layar.klik_di(
        Point::new(LAYAR.width - 8.0, LAYAR.height - 8.0),
        PointerButton::Primary,
    );
    assert!(!layar.keadaan().is_open());
    assert!(layar.dipilih.borrow().is_empty());
}

#[test]
fn keyboard_menyusuri_membuka_submenu_lalu_memilih_di_dalamnya() {
    let mut layar = Layar::baru(tema());

    // Tab reaches the trigger, Space opens the menu.
    layar.tekan(KeyCode::Named(NamedKey::Tab));
    layar.tekan(KeyCode::Named(NamedKey::Space));
    assert!(layar.keadaan().is_open());
    assert_eq!(layar.keadaan().highlight, None);

    // End jumps to the last selectable row — the submenu parent.
    layar.tekan(KeyCode::Named(NamedKey::End));
    assert_eq!(layar.keadaan().highlight, Some(URUTKAN));

    // → opens it. The panel appears only once its anchor has been measured, so
    // there is no frame in which it hangs somewhere it does not belong.
    layar.tekan(KeyCode::Named(NamedKey::ArrowRight));
    assert_eq!(layar.keadaan().depth(), 1);
    assert_eq!(layar.panel_terlihat().len(), 2, "dua panel terlihat");
    assert_eq!(layar.baris_tingkat(1).len(), 2);
    assert_eq!(layar.keadaan().highlight, Some(0));

    // The submenu really hangs beside its parent row, not on top of it.
    let induk = layar.kotak(layar.baris_ke(0, URUTKAN));
    let panel = layar.panel_terlihat();
    assert!(
        panel[1].min_x() >= induk.max_x() - 0.01 || panel[1].max_x() <= induk.min_x() + 0.01,
        "submenu {:?} harus di samping baris induk {:?}",
        panel[1],
        induk
    );

    // ↓ then Return chooses inside the submenu.
    layar.tekan(KeyCode::Named(NamedKey::ArrowDown));
    layar.tekan(KeyCode::Named(NamedKey::Enter));
    assert_eq!(layar.dipilih.borrow().as_slice(), ["sort.date"]);
    assert!(!layar.keadaan().is_open());
}

#[test]
fn esc_menutup_satu_tingkat_di_pohon_sungguhan() {
    let mut layar = Layar::baru(tema());
    layar.tekan(KeyCode::Named(NamedKey::Tab));
    layar.tekan(KeyCode::Named(NamedKey::Space));
    layar.tekan(KeyCode::Named(NamedKey::End));
    layar.tekan(KeyCode::Named(NamedKey::ArrowRight));
    assert_eq!(layar.panel_terlihat().len(), 2);

    layar.tekan(KeyCode::Named(NamedKey::Escape));
    assert_eq!(
        layar.panel_terlihat().len(),
        1,
        "cuma submenu yang tertutup"
    );
    assert!(layar.keadaan().is_open());

    layar.tekan(KeyCode::Named(NamedKey::Escape));
    assert!(!layar.keadaan().is_open());
    assert!(
        layar.dipilih.borrow().is_empty(),
        "Esc tidak memilih apa pun"
    );
}

#[test]
fn panah_kiri_kembali_dari_submenu() {
    let mut layar = Layar::baru(tema());
    layar.tekan(KeyCode::Named(NamedKey::Tab));
    layar.tekan(KeyCode::Named(NamedKey::Space));
    layar.tekan(KeyCode::Named(NamedKey::End));
    layar.tekan(KeyCode::Named(NamedKey::ArrowRight));
    assert_eq!(layar.keadaan().depth(), 1);

    layar.tekan(KeyCode::Named(NamedKey::ArrowLeft));
    assert_eq!(layar.keadaan().depth(), 0);
    assert_eq!(layar.keadaan().highlight, Some(URUTKAN));
    // ← at the root level does nothing: there is no menubar to walk into yet.
    layar.tekan(KeyCode::Named(NamedKey::ArrowLeft));
    assert!(layar.keadaan().is_open());
}

#[test]
fn mengetik_huruf_melompat_ke_baris_yang_cocok() {
    let mut layar = Layar::baru(tema());
    layar.buka_dengan_klik();

    layar.ketik('t');
    assert_eq!(layar.keadaan().highlight, Some(KISI));
    layar.ketik('t');
    assert_eq!(
        layar.keadaan().highlight,
        Some(PENGGARIS),
        "huruf yang sama berpindah ke kecocokan berikutnya"
    );
    // Return then chooses whatever the letters landed on.
    layar.tekan(KeyCode::Named(NamedKey::Enter));
    assert_eq!(layar.dipilih.borrow().as_slice(), ["view.ruler"]);
}

#[test]
fn menyorot_dengan_kursor_membuka_submenu_tanpa_klik() {
    let mut layar = Layar::baru(tema());
    layar.buka_dengan_klik();

    let induk = layar.kotak(layar.baris_ke(0, URUTKAN));
    layar.arahkan(induk.center());
    assert_eq!(layar.keadaan().depth(), 1, "hover membuka submenu");
    assert_eq!(
        layar.keadaan().highlight,
        None,
        "masuk lewat kursor tidak menyorot baris mana pun dulu"
    );

    // Moving to another row of the parent level closes it again.
    let lain = layar.kotak(layar.baris_ke(0, KISI));
    layar.arahkan(lain.center());
    assert_eq!(layar.keadaan().depth(), 0);
    assert_eq!(layar.panel_terlihat().len(), 1);
}

#[test]
fn klik_kanan_membuka_menu_di_posisi_kursor() {
    let mut layar = Layar::dengan(tema(), Rect::new(80.0, 100.0, 0.0, 0.0), true);
    let titik = Point::new(200.0, 160.0);
    layar.klik_di(titik, PointerButton::Secondary);

    let s = layar.keadaan();
    assert!(s.is_open(), "klik kanan membuka menu konteks");
    match s.anchor {
        Anchor::Point(p) => {
            assert!(
                (p.x - titik.x).abs() < 1.0 && (p.y - titik.y).abs() < 1.0,
                "menu konteks menempel di kursor, bukan di kotak wilayahnya: {p:?}"
            );
        }
        lain => panic!("jangkar menu konteks harus sebuah titik, bukan {lain:?}"),
    }
    // The panel hangs at that point and still stays on screen.
    let panel = layar.panel_terlihat();
    assert_eq!(panel.len(), 1);
    assert!(panel[0].min_y() >= titik.y - 0.01);
    assert!(panel[0].max_y() <= LAYAR.height + 0.01);

    // A11y: the region advertises that it has a context menu at all.
    let pohon = layar.a11y();
    assert!(pohon
        .entries()
        .iter()
        .any(|e| e.node.actions.contains(AccessActions::CONTEXT_MENU)));
}

#[test]
fn klik_kiri_di_wilayah_konteks_tidak_membuka_apa_pun() {
    let mut layar = Layar::dengan(tema(), Rect::new(80.0, 100.0, 0.0, 0.0), true);
    layar.klik_di(Point::new(90.0, 110.0), PointerButton::Primary);
    assert!(
        !layar.keadaan().is_open(),
        "wilayah konteks tidak boleh mencuri klik kiri isinya"
    );
}

#[test]
fn semua_nilai_gambar_berasal_dari_token_di_kedua_preset() {
    for preset in Preset::ALL {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let t = Theme::new(preset, appearance);
            let m = menu_in(&Fonts::bundled_only(), &t, isi());
            let pemicu = m.trigger_style();
            assert_eq!(pemicu.rest, t.color.surface);
            assert_eq!(pemicu.hover, t.color.surface_hover);
            assert_eq!(pemicu.focus_ring, t.color.focus_ring);
            assert_eq!(pemicu.corners.style, t.radius.style);
            assert!(pemicu.min_height >= crate::MIN_HIT_TARGET);

            let baris = m.row_style(0);
            assert_eq!(baris.highlight, t.color.surface_hover);
            assert_eq!(baris.mark, t.color.accent);
            assert_eq!(baris.corners.style, t.radius.style);
            assert!(baris.min_height >= crate::MIN_HIT_TARGET);
            // The level has both checkables and a submenu, so both gutters are
            // reserved; a level without them reserves neither.
            assert!(baris.leading > 0.0 && baris.trailing > 0.0);
            let polos = menu_in(&Fonts::bundled_only(), &t, [item("a", "A")]).row_style(0);
            assert_eq!((polos.leading, polos.trailing), (0.0, 0.0));

            // A chip differs only in its corner radius — nothing else forks.
            let chip = menu_in(&Fonts::bundled_only(), &t, isi())
                .chip(true)
                .trigger_style();
            assert_eq!(chip.corners.radii.max(), t.radius.full);
        }
    }
}

#[test]
fn halaman_diam_tidak_menyisakan_pekerjaan() {
    let mut layar = Layar::baru(tema());
    layar.diamkan();
    assert!(!crate::motion::is_animating(&layar.tree));
    // Opening and closing both come to rest — no spring left running, which is
    // what lets the GPU sleep (§3.5).
    layar.buka_dengan_klik();
    assert!(!crate::motion::is_animating(&layar.tree));
    layar.tekan(KeyCode::Named(NamedKey::Escape));
    assert!(!crate::motion::is_animating(&layar.tree));
}

#[test]
fn menu_kosong_tidak_membuat_apa_pun_panik() {
    let rt = Runtime::new();
    let state = rt.signal(MenuState::new());
    let fonts = Fonts::bundled_only();
    let t = tema();
    let m = menu_in(&fonts, &t, Vec::<MenuEntry>::new())
        .label("Kosong")
        .bind(state)
        .open(true);
    let mut layer = overlay_layer(column([m.trigger("Kosong")]));
    for panel in m.overlays() {
        layer = layer.overlay(panel);
    }
    let mut tree = RenderTree::new();
    reconcile(&mut tree, layer);
    tree.layout(BoxConstraints::tight(LAYAR));
    crate::motion::settle(&mut tree);
    tree.flush_layout();

    let mut s = state.peek();
    // Every navigation intent is a no-op instead of an index into nothing.
    assert!(!s.apply(MenuIntent::Move(1), m.model()));
    assert!(!s.apply(MenuIntent::First, m.model()));
    assert!(!s.apply(MenuIntent::Descend, m.model()));
    assert_eq!(s.highlight, None);
}

#[test]
fn gutter_dan_segitiga_tercermin_di_rtl() {
    let t = tema();
    let gaya = menu_in(&Fonts::bundled_only(), &t, isi()).row_style(0);
    let ltr = gaya.insets(false);
    let rtl = gaya.insets(true);
    // The mark gutter sits at the start of the line and the submenu triangle at
    // its end, so an Arabic UI swaps the two without a single value being
    // recomputed in the view layer (§9.8).
    assert_eq!(ltr.left - gaya.padding.left, gaya.leading);
    assert_eq!(ltr.right - gaya.padding.right, gaya.trailing);
    assert_eq!(rtl.right - gaya.padding.right, gaya.leading);
    assert_eq!(rtl.left - gaya.padding.left, gaya.trailing);
    // The row is exactly as wide either way: mirroring moves space, never adds.
    assert_eq!(ltr.horizontal(), rtl.horizontal());
}

#[test]
fn reduced_motion_tetap_membuka_menu_dan_tetap_berhenti() {
    let mut layar = Layar::baru(tema());
    layar.gerak = Motion::Reduced;

    layar.buka_dengan_klik();
    assert!(layar.keadaan().is_open());
    // The motion is `Essential`, so reduced motion drops the bounce but keeps
    // the panel arriving — and it still comes to rest, which is what lets the
    // GPU sleep afterwards.
    assert!(!crate::motion::is_animating(&layar.tree));
    assert_eq!(layar.panel_terlihat().len(), 1);

    layar.tekan(KeyCode::Named(NamedKey::End));
    layar.tekan(KeyCode::Named(NamedKey::ArrowRight));
    assert_eq!(
        layar.panel_terlihat().len(),
        2,
        "submenu tetap terbuka walau gerak dikurangi"
    );
}
