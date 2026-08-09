//! Uji `dialog()` sebagai satu kesatuan.
//!
//! [`crate::overlay`] sudah menguji geometri, backdrop, dismiss, dan transisi
//! spring-nya sendiri; yang diuji di sini adalah **apa yang ditambahkan
//! dialog**: konvensi tombol per-OS, tombol default keyboard, aksi batal yang
//! tersambung ke Esc, dan Definition of Done `KOMPONEN.md` (kedua preset, dark
//! mode, a11y, hit target, reduced-motion).

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use rustui_core::access::{AccessActions, AccessRole};
use rustui_core::animation::{Motion, Spring, Tick};
use rustui_core::input::{
    Event, InputRouter, KeyCode, KeyEvent, NamedKey, PointerButton, PointerEvent, PointerPhase,
};
use rustui_core::scheduler::Dirty;
use rustui_core::tree::{BoxConstraints, NodeId, RenderTree};
use rustui_core::view::{fixed, reconcile, View};
use rustui_paint::{Color, Command, Point, Scene, Size};
use rustui_theme::{Appearance, Preset};

use super::*;
use crate::motion::{advance, settle};
use crate::overlay::{dismiss_topmost, entries, overlay_layer, OverlayEntry};

const LAYAR: Size = Size::new(640.0, 480.0);

fn fonts() -> Fonts {
    // Tanpa font sistem: hasil test tidak tergantung font apa yang kebetulan
    // terpasang di mesin CI (§9.5).
    Fonts::bundled_only()
}

fn tema() -> Theme {
    Theme::cupertino(Appearance::Dark)
}

fn pohon(view: impl Into<View>) -> RenderTree {
    let mut tree = RenderTree::new();
    reconcile(&mut tree, view);
    tree.layout(BoxConstraints::tight(LAYAR));
    tree
}

/// Dialog terbuka di atas konten seukuran layar, sudah selesai beranimasi.
fn buka(d: DialogBuilder) -> RenderTree {
    let mut tree = pohon(overlay_layer(fixed(LAYAR.width, LAYAR.height)).overlay(d.open(true)));
    settle(&mut tree);
    tree.flush_layout();
    tree
}

fn entri(tree: &RenderTree) -> NodeId {
    *entries(tree).first().expect("harus ada satu overlay")
}

fn panel(tree: &RenderTree) -> rustui_paint::Rect {
    tree.node_ref::<OverlayEntry>(entri(tree))
        .expect("overlay")
        .panel_rect()
}

fn scene(tree: &mut RenderTree) -> Scene {
    let mut s = Scene::new(Color::TRANSPARENT);
    tree.paint_into(&mut s);
    s
}

fn tombol_key(code: NamedKey) -> Event {
    Event::Key(KeyEvent::pressed(KeyCode::Named(code), Duration::ZERO))
}

fn tekan(pos: Point) -> Event {
    let mut e =
        PointerEvent::new(PointerPhase::Down, pos, Duration::ZERO).button(PointerButton::Primary);
    e.buttons.insert(PointerButton::Primary);
    Event::Pointer(e)
}

fn lepas(pos: Point) -> Event {
    Event::Pointer(
        PointerEvent::new(PointerPhase::Up, pos, Duration::from_millis(10))
            .button(PointerButton::Primary),
    )
}

/// Pencacah yang bisa dititipkan ke sebuah aksi.
fn cacah() -> (Rc<Cell<u32>>, impl Fn() + Clone) {
    let n = Rc::new(Cell::new(0u32));
    let tulis = {
        let n = n.clone();
        move || n.set(n.get() + 1)
    };
    (n, tulis)
}

// ---------------------------------------------------------------------------
// Konvensi per-OS
// ---------------------------------------------------------------------------

fn nama(actions: &[DialogAction]) -> Vec<&str> {
    actions.iter().map(DialogAction::label).collect()
}

#[test]
fn urutan_tombol_mengikuti_konvensi_per_os() {
    let ditulis = || {
        vec![
            action("Simpan").confirm(),
            action("Batal").cancel(),
            action("Jangan Simpan"),
        ]
    };

    // macOS/GNOME: default paling kanan, batal di kirinya, sisanya paling kiri.
    let mac = ButtonOrder::ConfirmLast.arrange(ditulis());
    assert_eq!(nama(&mac), ["Jangan Simpan", "Batal", "Simpan"]);

    // Windows: persis cerminnya.
    let win = ButtonOrder::ConfirmFirst.arrange(ditulis());
    assert_eq!(nama(&win), ["Simpan", "Batal", "Jangan Simpan"]);
}

#[test]
fn yang_bertukar_tempat_antar_os_adalah_kelompok_bukan_tiap_tombol() {
    // Dua aksi "lainnya" ditulis A lalu B. Di kedua konvensi, kelompok
    // "lainnya" boleh pindah sisi, tapi A tetap dibaca sebelum B — kalau
    // seluruh vektor dibalik, Windows akan menampilkan B sebelum A.
    let ditulis = || {
        vec![
            action("A"),
            action("B"),
            action("Batal").cancel(),
            action("Simpan").confirm(),
        ]
    };

    let mac = ButtonOrder::ConfirmLast.arrange(ditulis());
    assert_eq!(nama(&mac), ["A", "B", "Batal", "Simpan"]);

    let win = ButtonOrder::ConfirmFirst.arrange(ditulis());
    assert_eq!(nama(&win), ["Simpan", "Batal", "A", "B"]);
}

#[test]
fn aksi_merusak_menempati_posisi_tombol_utama() {
    let urut = ButtonOrder::ConfirmLast.arrange(vec![
        action("Hapus").destructive(),
        action("Batal").cancel(),
    ]);
    assert_eq!(nama(&urut), ["Batal", "Hapus"]);
}

#[test]
fn susunan_bawaan_datang_dari_platform_bukan_dari_pemanggil() {
    assert_eq!(ButtonOrder::default(), ButtonOrder::Platform);
    assert_eq!(ButtonOrder::Platform.resolved(), ButtonOrder::PLATFORM);
    assert_eq!(
        ButtonOrder::ConfirmFirst.resolved(),
        ButtonOrder::ConfirmFirst
    );

    let harapan = if cfg!(target_os = "windows") {
        ButtonOrder::ConfirmFirst
    } else {
        ButtonOrder::ConfirmLast
    };
    assert_eq!(ButtonOrder::PLATFORM, harapan);

    // Dan builder-nya memang memakainya.
    let f = fonts();
    let t = tema();
    let d = dialog(&f, &t, "Judul")
        .confirm("Simpan", || {})
        .cancel("Batal", || {});
    assert_eq!(nama(&d.arranged()), nama(&harapan.arrange(d.arranged())));
}

#[test]
fn urutan_visual_adalah_urutan_tab() {
    // Tab menyusuri pohon apa adanya, jadi urutan tombol di layar **adalah**
    // urutan fokus: di macOS Tab pertama jatuh ke Batal, bukan ke Simpan.
    let f = fonts();
    let t = tema();
    let tree = buka(
        dialog(&f, &t, "Simpan perubahan?")
            .order(ButtonOrder::ConfirmLast)
            .confirm("Simpan", || {})
            .cancel("Batal", || {}),
    );

    let a11y = tree.access_tree(None);
    let urut: Vec<String> = rustui_core::input::tab_order(&tree, entri(&tree))
        .into_iter()
        .filter_map(|id| {
            a11y.entries()
                .iter()
                .find(|e| e.id == id)?
                .node
                .label
                .clone()
        })
        .collect();
    assert_eq!(urut, ["Batal", "Simpan"], "{}", a11y.dump());
}

// ---------------------------------------------------------------------------
// Bentuk & tata letak
// ---------------------------------------------------------------------------

#[test]
fn panel_berada_di_tengah_layer_dengan_lebar_token() {
    let f = fonts();
    let t = tema();
    let tree = buka(
        dialog(&f, &t, "Simpan perubahan?")
            .message("Perubahan yang belum disimpan akan hilang.")
            .confirm("Simpan", || {})
            .cancel("Batal", || {}),
    );

    let p = panel(&tree);
    assert_eq!(p.size.width, t.space(DIALOG_WIDTH_STEPS));
    assert!(p.size.height > 0.0);
    // Tepat di tengah, dan seluruhnya di dalam layar.
    assert!((p.center().x - LAYAR.width / 2.0).abs() < 0.5, "{p:?}");
    assert!((p.center().y - LAYAR.height / 2.0).abs() < 0.5, "{p:?}");
    assert!(p.min_x() >= 0.0 && p.max_x() <= LAYAR.width);
}

#[test]
fn panel_menyempit_di_window_yang_lebih_sempit_dari_dialognya() {
    let f = fonts();
    let t = tema();
    let sempit = Size::new(240.0, 320.0);
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        overlay_layer(fixed(sempit.width, sempit.height)).overlay(
            dialog(&f, &t, "Judul")
                .message("Pesan yang cukup panjang supaya harus dibungkus.")
                .open(true)
                .confirm("Ok", || {}),
        ),
    );
    tree.layout(BoxConstraints::tight(sempit));
    settle(&mut tree);
    tree.flush_layout();

    let p = panel(&tree);
    assert!(p.size.width <= sempit.width, "{p:?}");
    assert!(p.max_x() <= sempit.width + 0.5, "{p:?}");
}

#[test]
fn dialog_tanpa_aksi_tidak_menyisakan_baris_tombol_kosong() {
    let f = fonts();
    let t = tema();
    let tree = buka(dialog(&f, &t, "Menghubungkan…").message("Mohon tunggu."));
    let a11y = tree.access_tree(None);
    assert!(
        !a11y
            .entries()
            .iter()
            .any(|e| e.node.role == AccessRole::Button),
        "{}",
        a11y.dump()
    );
}

// ---------------------------------------------------------------------------
// Keyboard
// ---------------------------------------------------------------------------

/// Kontrol yang bisa difokuskan tapi membiarkan **semua** tombol lewat.
///
/// Berdiri di tempat `text_field` (`KOMPONEN.md` Tier 2, belum ada) untuk satu
/// pertanyaan yang tidak bisa dijawab tanpa dia: kalau fokus berada di sebuah
/// kontrol yang tidak menelan Return, apakah Return sampai ke tombol default
/// dialog? Tombol biasa tidak bisa dipakai untuk menanyakannya karena ia
/// memang **harus** menelan Return untuk dirinya sendiri.
#[derive(Debug, Default)]
struct KolomPalsu {
    label: String,
}

impl rustui_core::tree::RenderNode for KolomPalsu {
    fn layout(
        &mut self,
        _ctx: &mut rustui_core::tree::LayoutCtx<'_>,
        constraints: BoxConstraints,
    ) -> Size {
        constraints.constrain(Size::new(200.0, 28.0))
    }

    fn access(&self, node: &mut rustui_core::access::AccessNode) {
        node.role = AccessRole::TextInput;
        node.label = Some(self.label.clone());
        node.actions |= AccessActions::FOCUS;
    }

    fn focus_policy(&self) -> rustui_core::input::FocusPolicy {
        rustui_core::input::FocusPolicy::FOCUSABLE
    }
}

#[derive(Debug, Clone, PartialEq)]
struct KolomPalsuProps {
    label: String,
}

impl rustui_core::view::ViewNode for KolomPalsuProps {
    fn build(&self) -> Box<dyn rustui_core::tree::RenderNode> {
        Box::new(KolomPalsu {
            label: self.label.clone(),
        })
    }

    fn update(&self, _node: &mut dyn rustui_core::tree::RenderNode) -> Dirty {
        Dirty::NONE
    }
}

fn kolom_palsu(label: &str) -> View {
    rustui_core::view::Builder::new(KolomPalsuProps {
        label: label.to_string(),
    })
    .into()
}

/// Node [`DialogPanel`] di dalam pohon — titik masuk uji jalur Return.
fn panel_node(tree: &RenderTree) -> NodeId {
    fn cari(tree: &RenderTree, id: NodeId) -> Option<NodeId> {
        if tree.node_ref::<DialogPanel>(id).is_some() {
            return Some(id);
        }
        tree.children(id).iter().find_map(|a| cari(tree, *a))
    }
    cari(tree, tree.root()).expect("dialog harus punya panel")
}

#[test]
fn return_menjalankan_tombol_default_dari_kontrol_lain_di_dalam_dialog() {
    let f = fonts();
    let t = tema();
    let (simpan, tulis_simpan) = cacah();
    let (batal, tulis_batal) = cacah();
    // Kontrol yang bisa difokuskan tapi **tidak** menelan Return — berdiri di
    // sini mewakili kolom teks satu baris di dalam dialog (`text_field` belum
    // ada; lihat `KolomPalsu`).
    let kolom = kolom_palsu("Nama berkas");
    let mut tree = buka(
        dialog(&f, &t, "Simpan perubahan?")
            .content(kolom)
            .confirm("Simpan", tulis_simpan)
            .cancel("Batal", tulis_batal),
    );

    let kolom_id = tree
        .access_tree(None)
        .find_label("Nama berkas")
        .expect("kolom isian")
        .id;
    let mut router = InputRouter::new();
    router.focus_node(&mut tree, Some(kolom_id));
    assert!(
        router
            .dispatch(&mut tree, &tombol_key(NamedKey::Enter))
            .handled
    );
    assert_eq!(simpan.get(), 1, "Return dari dalam dialog = tombol default");
    assert_eq!(batal.get(), 0);
}

#[test]
fn tombol_yang_terfokus_menang_atas_tombol_default() {
    // Return diberikan lebih dulu ke node terfokus; yang menekan Return sambil
    // fokus ada di "Batal" memang bermaksud membatalkan.
    let f = fonts();
    let t = tema();
    let (simpan, tulis_simpan) = cacah();
    let (batal, tulis_batal) = cacah();
    let mut tree = buka(
        dialog(&f, &t, "Simpan perubahan?")
            .confirm("Simpan", tulis_simpan)
            .cancel("Batal", tulis_batal),
    );

    let id = tree
        .access_tree(None)
        .find_label("Batal")
        .expect("tombol Batal")
        .id;
    let mut router = InputRouter::new();
    router.focus_node(&mut tree, Some(id));
    assert!(
        router
            .dispatch(&mut tree, &tombol_key(NamedKey::Enter))
            .handled
    );
    assert_eq!(batal.get(), 1);
    assert_eq!(simpan.get(), 0);
}

#[test]
fn return_tanpa_fokus_ditangani_jaring_pengaman() {
    let f = fonts();
    let t = tema();
    let (simpan, tulis) = cacah();
    let mut tree = buka(dialog(&f, &t, "Simpan?").confirm("Simpan", tulis));

    let mut router = InputRouter::new();
    // Tanpa fokus, event tombol hanya sampai ke akar pohon…
    assert!(
        !router
            .dispatch(&mut tree, &tombol_key(NamedKey::Enter))
            .handled
    );
    assert_eq!(simpan.get(), 0);
    // …dan di situlah `activate_default` ada.
    assert!(activate_default(&mut tree));
    assert_eq!(simpan.get(), 1);
}

#[test]
fn return_tidak_menyentuh_dialog_yang_tertutup() {
    let f = fonts();
    let t = tema();
    let (simpan, tulis) = cacah();
    let mut tree = pohon(
        overlay_layer(fixed(LAYAR.width, LAYAR.height)).overlay(
            dialog(&f, &t, "Simpan?")
                .open(false)
                .confirm("Simpan", tulis),
        ),
    );
    assert!(!activate_default(&mut tree));
    assert_eq!(simpan.get(), 0);
}

#[test]
fn aksi_merusak_tidak_pernah_dijalankan_return() {
    // HIG: aksi merusak tidak boleh punya tombol default — Return yang
    // terlanjur ditekan tidak boleh menghapus apa pun.
    let f = fonts();
    let t = tema();
    let (hapus, tulis_hapus) = cacah();
    let (batal, tulis_batal) = cacah();
    let mut tree = buka(
        alert(&f, &t, "Hapus 3 berkas?")
            .destructive("Hapus", tulis_hapus)
            .cancel("Batal", tulis_batal),
    );

    let mut router = InputRouter::new();
    let panel = panel_node(&tree);
    router.focus_node(&mut tree, Some(panel));
    assert!(
        !router
            .dispatch(&mut tree, &tombol_key(NamedKey::Enter))
            .handled
    );
    assert!(!activate_default(&mut tree));
    assert_eq!(hapus.get(), 0);
    assert_eq!(batal.get(), 0);
}

#[test]
fn esc_menjalankan_aksi_batal() {
    let f = fonts();
    let t = tema();
    let (batal, tulis) = cacah();
    let mut tree = buka(
        dialog(&f, &t, "Simpan perubahan?")
            .confirm("Simpan", || {})
            .cancel("Batal", tulis),
    );

    let mut router = InputRouter::new();
    let dialog_id = entri(&tree);
    router.focus_node(&mut tree, Some(dialog_id));
    assert!(
        router
            .dispatch(&mut tree, &tombol_key(NamedKey::Escape))
            .handled
    );
    assert_eq!(batal.get(), 1, "Esc harus sama dengan menekan Batal");
}

#[test]
fn on_dismiss_eksplisit_mendahului_aksi_batal() {
    let f = fonts();
    let t = tema();
    let (batal, tulis_batal) = cacah();
    let (tutup, tulis_tutup) = cacah();
    let mut tree = buka(
        dialog(&f, &t, "Judul")
            .cancel("Batal", tulis_batal)
            .on_dismiss(tulis_tutup),
    );

    assert!(dismiss_topmost(&mut tree, Dismiss::ESCAPE));
    assert_eq!(tutup.get(), 1);
    assert_eq!(batal.get(), 0);
}

#[test]
fn tombol_bisa_diaktifkan_keyboard_lewat_space() {
    let f = fonts();
    let t = tema();
    let (simpan, tulis) = cacah();
    let mut tree = buka(dialog(&f, &t, "Simpan?").confirm("Simpan", tulis));

    let mut router = InputRouter::new();
    // Tab pertama mendarat di dialognya sendiri (tempat mendarat sebuah modal,
    // lihat `Barrier::Modal`), Tab kedua masuk ke tombol pertama di dalamnya,
    // lalu Space mengaktifkannya — keyboard bukan warga kelas dua
    // (`KOMPONEN.md` DoD).
    router.dispatch(&mut tree, &tombol_key(NamedKey::Tab));
    assert_eq!(router.focus().focused(), Some(entri(&tree)));
    router.dispatch(&mut tree, &tombol_key(NamedKey::Tab));
    router.dispatch(&mut tree, &tombol_key(NamedKey::Space));
    assert_eq!(simpan.get(), 1);
}

// ---------------------------------------------------------------------------
// Dismiss dengan penunjuk
// ---------------------------------------------------------------------------

#[test]
fn klik_di_luar_menutup_dialog_tapi_tidak_menutup_alert() {
    let f = fonts();
    let t = tema();
    let luar = Point::new(8.0, 8.0);

    let (n, tulis) = cacah();
    let mut tree = buka(dialog(&f, &t, "Judul").cancel("Batal", tulis));
    let mut router = InputRouter::new();
    router.dispatch(&mut tree, &tekan(luar));
    router.dispatch(&mut tree, &lepas(luar));
    assert_eq!(n.get(), 1, "dialog shadcn: klik luar = batal");

    let (n, tulis) = cacah();
    let mut tree = buka(alert(&f, &t, "Hapus?").cancel("Batal", tulis));
    let mut router = InputRouter::new();
    router.dispatch(&mut tree, &tekan(luar));
    router.dispatch(&mut tree, &lepas(luar));
    assert_eq!(
        n.get(),
        0,
        "NSAlert tidak boleh hilang karena kursor tergelincir"
    );
    // …tapi Esc tetap berlaku.
    assert!(dismiss_topmost(&mut tree, Dismiss::ESCAPE));
    assert_eq!(n.get(), 1);
}

#[test]
fn klik_tombol_menjalankan_aksinya_lewat_lapisan_input() {
    let f = fonts();
    let t = tema();
    let (simpan, tulis) = cacah();
    let mut tree = buka(dialog(&f, &t, "Simpan?").confirm("Simpan", tulis));

    let kotak = tree
        .access_tree(None)
        .find_label("Simpan")
        .expect("tombol Simpan")
        .bounds;
    let mut router = InputRouter::new();
    router.dispatch(&mut tree, &tekan(kotak.center()));
    router.dispatch(&mut tree, &lepas(kotak.center()));
    assert_eq!(simpan.get(), 1);
}

// ---------------------------------------------------------------------------
// Aksesibilitas
// ---------------------------------------------------------------------------

#[test]
fn dialog_punya_peran_nama_dan_aksi_bagi_screen_reader() {
    let f = fonts();
    let t = tema();
    let tree = buka(
        dialog(&f, &t, "Simpan perubahan?")
            .message("Perubahan yang belum disimpan akan hilang.")
            .confirm("Simpan", || {})
            .cancel("Batal", || {}),
    );
    let a11y = tree.access_tree(None);

    let d = a11y
        .find_label("Simpan perubahan?")
        .unwrap_or_else(|| panic!("{}", a11y.dump()));
    assert_eq!(d.node.role, AccessRole::Dialog);

    // Judul dibacakan **sekali**: sebagai nama dialog, bukan lagi sebagai teks.
    let jumlah = a11y
        .entries()
        .iter()
        .filter(|e| e.node.label.as_deref() == Some("Simpan perubahan?"))
        .count();
    assert_eq!(jumlah, 1, "judul dibacakan dua kali:\n{}", a11y.dump());

    // Pesannya tetap bisa dibaca saat menyusuri isi dialog.
    let pesan = a11y
        .find_label("Perubahan yang belum disimpan akan hilang.")
        .unwrap_or_else(|| panic!("{}", a11y.dump()));
    assert_eq!(pesan.node.role, AccessRole::Label);

    for label in ["Simpan", "Batal"] {
        let b = a11y
            .find_label(label)
            .unwrap_or_else(|| panic!("{label} hilang:\n{}", a11y.dump()));
        assert_eq!(b.node.role, AccessRole::Button);
        assert!(b.node.actions.contains(AccessActions::CLICK));
        assert!(b.node.actions.contains(AccessActions::FOCUS));
    }
}

#[test]
fn konten_di_belakang_dialog_benar_benar_inert() {
    let f = fonts();
    let t = tema();
    let konten = rustui_core::view::interactive(fixed(120.0, 44.0)).label("Di belakang");
    let mut tree = pohon(
        overlay_layer(konten).overlay(dialog(&f, &t, "Judul").open(true).confirm("Ok", || {})),
    );
    settle(&mut tree);
    tree.flush_layout();

    let a11y = tree.access_tree(None);
    assert!(
        a11y.find_label("Di belakang").is_none(),
        "konten di belakang modal masih dibacakan:\n{}",
        a11y.dump()
    );
    assert!(rustui_core::input::tab_order(&tree, tree.root())
        .iter()
        .all(|id| *id != tree.children(tree.root())[0]));
}

#[test]
fn tombol_mati_dibacakan_dimmed_dan_tidak_bisa_diklik() {
    let f = fonts();
    let t = tema();
    let (n, tulis) = cacah();
    let mut tree = buka(
        dialog(&f, &t, "Judul").action(action("Simpan").confirm().on_press(tulis).disabled(true)),
    );

    let a11y = tree.access_tree(None);
    let b = a11y
        .find_label("Simpan")
        .unwrap_or_else(|| panic!("{}", a11y.dump()));
    assert!(b.node.disabled);
    assert!(!b.node.actions.contains(AccessActions::CLICK));

    // Termasuk lewat tombol default: aksi yang mati tidak punya callback.
    assert!(!activate_default(&mut tree));
    let mut router = InputRouter::new();
    router.dispatch(&mut tree, &tekan(b.bounds.center()));
    router.dispatch(&mut tree, &lepas(b.bounds.center()));
    assert_eq!(n.get(), 0);
}

#[test]
fn hit_target_setiap_tombol_minimal_44pt() {
    let f = fonts();
    let t = tema();
    let tree = buka(
        dialog(&f, &t, "Judul")
            .confirm("Ok", || {})
            .cancel("Batal", || {}),
    );
    let a11y = tree.access_tree(None);
    for label in ["Ok", "Batal"] {
        let b = a11y.find_label(label).expect("tombol");
        assert!(
            b.bounds.size.height >= crate::MIN_HIT_TARGET,
            "hit target {label} cuma {:?}",
            b.bounds.size
        );
    }
}

// ---------------------------------------------------------------------------
// Token: kedua preset + dark mode
// ---------------------------------------------------------------------------

#[test]
fn seluruh_warna_dan_sudut_datang_dari_token_di_kedua_preset() {
    let f = fonts();
    for preset in Preset::ALL {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let t = Theme::new(preset, appearance);
            let mut tree = buka(
                dialog(&f, &t, "Simpan perubahan?")
                    .message("Perubahan yang belum disimpan akan hilang.")
                    .confirm("Simpan", || {})
                    .cancel("Batal", || {}),
            );
            let s = scene(&mut tree);
            let kotak: Vec<_> = s
                .commands()
                .iter()
                .filter_map(|c| match c {
                    Command::Quad(q) => Some(q.clone()),
                    _ => None,
                })
                .collect();

            // Peredup: satu quad seukuran layer, warnanya token `scrim`.
            assert!(
                kotak
                    .iter()
                    .any(|q| q.rect.size == LAYAR && q.background == t.color.scrim),
                "{preset:?}/{appearance:?}: backdrop bukan token scrim"
            );
            // Panel: permukaan terangkat, sudut mengikuti bentuk preset.
            let p = panel(&tree);
            let kartu = kotak
                .iter()
                .find(|q| q.rect.size == p.size)
                .unwrap_or_else(|| panic!("{preset:?}/{appearance:?}: panel tidak tergambar"));
            assert_eq!(kartu.background, t.color.surface_elevated);
            assert_eq!(kartu.corners.style, t.radius.style);
            assert_eq!(kartu.corners.radii.max(), t.radius.xl);
            assert_eq!(kartu.border_color, t.color.separator);

            // Tombol utama memakai aksen, dan tidak ada warna teks yang lepas
            // dari token.
            assert!(kotak.iter().any(|q| q.background == t.color.accent));
            for c in s.commands() {
                if let Command::GlyphRun(r) = c {
                    assert!(
                        r.color == t.color.label
                            || r.color == t.color.secondary_label
                            || r.color == t.color.on_accent,
                        "warna teks lepas dari token: {:?} ({preset:?}/{appearance:?})",
                        r.color
                    );
                }
            }
        }
    }
}

#[test]
fn dark_mode_mengganti_warna_panel_tanpa_menggeser_geometrinya() {
    let f = fonts();
    let ukur = |appearance| {
        let t = Theme::cupertino(appearance);
        let mut tree = buka(
            dialog(&f, &t, "Judul")
                .message("Pesan.")
                .confirm("Ok", || {}),
        );
        let p = panel(&tree);
        let latar = scene(&mut tree)
            .commands()
            .iter()
            .find_map(|c| match c {
                Command::Quad(q) if q.rect.size == p.size => Some(q.background),
                _ => None,
            })
            .expect("panel");
        (p, latar)
    };
    let (terang_rect, terang) = ukur(Appearance::Light);
    let (gelap_rect, gelap) = ukur(Appearance::Dark);

    assert_ne!(terang, gelap, "panel harus ikut dark mode");
    assert_eq!(
        terang_rect, gelap_rect,
        "yang berubah saat matahari terbenam hanya warna, bukan tata letak"
    );
}

// ---------------------------------------------------------------------------
// Transisi spring
// ---------------------------------------------------------------------------

/// Majukan seluruh transisi sampai diam, seperti siklus frame sungguhan.
fn sampai_diam(tree: &mut RenderTree, motion: Motion) -> u32 {
    let tick = Tick::manual(Duration::from_millis(16), motion);
    let mut frame = 0;
    while advance(tree, &tick).contains(Dirty::ANIMATION) {
        tree.flush_layout();
        frame += 1;
        assert!(frame < 600, "spring tidak pernah settle");
    }
    tree.flush_layout();
    frame
}

#[test]
fn dialog_yang_baru_terbuka_bergerak_alih_alih_melompat() {
    let f = fonts();
    let t = tema();
    let mut tree = pohon(
        overlay_layer(fixed(LAYAR.width, LAYAR.height))
            .overlay(dialog(&f, &t, "Judul").open(true).confirm("Ok", || {})),
    );
    let id = entri(&tree);
    let mulai = panel(&tree).origin;
    assert_eq!(tree.node_ref::<OverlayEntry>(id).unwrap().progress(), 0.0);

    let frame = sampai_diam(&mut tree, Motion::Full);
    assert!(frame > 1, "transisi harus memakan lebih dari satu frame");
    assert_ne!(mulai, panel(&tree).origin, "panel harus bergerak");
    assert_eq!(tree.node_ref::<OverlayEntry>(id).unwrap().progress(), 1.0);
}

#[test]
fn menutup_di_tengah_animasi_buka_membawa_kecepatan() {
    let f = fonts();
    let t = tema();
    let mut tree = pohon(
        overlay_layer(fixed(LAYAR.width, LAYAR.height))
            .overlay(dialog(&f, &t, "Judul").open(true).confirm("Ok", || {})),
    );
    let id = entri(&tree);
    let tick = Tick::manual(Duration::from_millis(16), Motion::Full);
    advance(&mut tree, &tick);
    advance(&mut tree, &tick);

    let e = tree.node_mut_ref::<OverlayEntry>(id).unwrap();
    let kemajuan = e.progress();
    assert!(kemajuan > 0.0 && kemajuan < 1.0);
    e.set_open(false);
    // Retarget, bukan animasi baru: posisinya tidak melompat ke nol.
    assert_eq!(e.progress(), kemajuan);

    sampai_diam(&mut tree, Motion::Full);
    let e = tree.node_ref::<OverlayEntry>(id).unwrap();
    assert_eq!(e.progress(), 0.0);
    assert!(!e.is_visible());
}

#[test]
fn reduced_motion_membuang_pantulan_tanpa_membuang_gerakan() {
    let f = fonts();
    let t = tema();
    let jalankan = |motion| {
        let mut tree = pohon(
            overlay_layer(fixed(LAYAR.width, LAYAR.height)).overlay(
                dialog(&f, &t, "Judul")
                    // Spring paling memantul: kalau reduced-motion benar, yang
                    // ini pun tidak boleh melewati tujuannya.
                    .spring(Spring::bouncy())
                    .open(true)
                    .confirm("Ok", || {}),
            ),
        );
        let id = entri(&tree);
        let tick = Tick::manual(Duration::from_millis(16), motion);
        let mut puncak: f32 = 0.0;
        let mut frame = 0;
        while advance(&mut tree, &tick).contains(Dirty::ANIMATION) {
            tree.flush_layout();
            puncak = puncak.max(tree.node_ref::<OverlayEntry>(id).unwrap().progress());
            frame += 1;
            assert!(frame < 600, "spring tidak pernah settle");
        }
        (
            frame,
            puncak,
            tree.node_ref::<OverlayEntry>(id).unwrap().progress(),
        )
    };

    let (frame_penuh, puncak_penuh, akhir_penuh) = jalankan(Motion::Full);
    assert!(puncak_penuh > 1.0, "spring bouncy harus melewati tujuannya");
    assert_eq!(akhir_penuh, 1.0);

    let (frame_reduced, puncak_reduced, akhir_reduced) = jalankan(Motion::Reduced);
    assert!(
        puncak_reduced <= 1.0 + 1e-4,
        "reduced-motion tidak boleh memantul, dapat {puncak_reduced}"
    );
    assert!(
        frame_reduced > 1,
        "gerakan yang menjelaskan tetap dipertahankan, bukan dihapus"
    );
    assert_eq!(akhir_reduced, 1.0);
    let _ = frame_penuh;
}
