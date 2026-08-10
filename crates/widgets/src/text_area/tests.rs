//! `text_area` tests — driven **through the input layer**, never by poking at
//! internal methods.
//!
//! The reasoning is `text_field`'s: what has to be proven is not "this function
//! returns that value" but "a user who types, clicks, or walks the arrow keys
//! gets the right result". Everything purely Unicode (graphemes, words, undo,
//! preedit) is already tested in `silka_text::edit`, and the shared keymap in
//! `crate::editing`; what is tested here is what makes an area **multi-line**:
//! wrapping, vertical navigation with a goal column, the gutter, auto-grow, the
//! seam to the scroll view, and the accessibility node.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use silka_core::access::{AccessActions, AccessRole};
use silka_core::animation::{Motion, Tick};
use silka_core::input::{
    Event, FocusDirection, ImeEvent, InputRouter, KeyCode, KeyEvent, Modifiers, NamedKey,
    PointerButton, PointerEvent, PointerPhase, Response,
};
use silka_core::tree::{BoxConstraints, NodeId, RenderTree};
use silka_core::view::{reconcile, View};
use silka_paint::{Command, Point, Scene, Size};
use silka_theme::{Appearance, Preset, Theme};

use super::*;
use crate::fonts::Fonts;
use crate::scroll_view::ScrollView;

const RUANG: Size = Size::new(320.0, 260.0);

/// Four short lines: enough for vertical navigation, short enough to reason
/// about.
const EMPAT_BARIS: &str = "satu\ndua\ntiga\nempat";

fn fonts() -> Fonts {
    Fonts::bundled_only()
}

fn tema() -> Theme {
    Theme::cupertino(Appearance::Dark)
}

/// One text area inside a tree, wired up to an input router.
struct Uji {
    tree: RenderTree,
    router: InputRouter,
    id: NodeId,
    jam: Duration,
}

impl Uji {
    fn baru(view: impl Into<View>) -> Self {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::loose(RUANG));
        let id = first(&tree).expect("badan text_area harus ada di pohon");
        Self {
            tree,
            router: InputRouter::new(),
            id,
            jam: Duration::ZERO,
        }
    }

    fn badan(&self) -> &TextAreaBody {
        self.tree.node_ref::<TextAreaBody>(self.id).expect("badan")
    }

    fn bingkai(&self) -> NodeId {
        frames(&self.tree)[0]
    }

    fn gulir(&self) -> &ScrollView {
        let id = crate::scroll_view::nodes(&self.tree)[0];
        self.tree.node_ref::<ScrollView>(id).expect("scroll view")
    }

    fn teks(&self) -> String {
        self.badan().text().to_string()
    }

    fn tinggi(&self) -> f32 {
        self.tree.size(self.bingkai()).height
    }

    fn maju(&mut self, ms: u64) -> Duration {
        self.jam += Duration::from_millis(ms);
        self.jam
    }

    fn fokus(&mut self) -> Response {
        let id = self.id;
        self.router.focus_node(&mut self.tree, Some(id))
    }

    fn tombol(&mut self, code: KeyCode, modifiers: Modifiers) -> Response {
        let t = self.maju(20);
        let e = KeyEvent::pressed(code, t).modifiers(modifiers);
        let r = self.router.dispatch(&mut self.tree, &Event::Key(e));
        self.tata();
        r
    }

    fn nama(&mut self, key: NamedKey, modifiers: Modifiers) -> Response {
        self.tombol(KeyCode::Named(key), modifiers)
    }

    fn ketik(&mut self, teks: &str) {
        for c in teks.chars() {
            match c {
                ' ' => self.nama(NamedKey::Space, Modifiers::NONE),
                '\n' => self.nama(NamedKey::Enter, Modifiers::NONE),
                c => self.tombol(KeyCode::Character(c), Modifiers::NONE),
            };
        }
    }

    fn ime(&mut self, e: ImeEvent) -> Response {
        let r = self.router.dispatch(&mut self.tree, &Event::Ime(e));
        self.tata();
        r
    }

    fn tekan(&mut self, titik: Point) {
        let t = self.maju(300);
        self.router.dispatch(
            &mut self.tree,
            &Event::Pointer(
                PointerEvent::new(PointerPhase::Down, titik, t).button(PointerButton::Primary),
            ),
        );
        self.tata();
    }

    fn seret(&mut self, titik: Point) {
        let t = self.maju(16);
        let mut e = PointerEvent::new(PointerPhase::Move, titik, t);
        e.buttons.insert(PointerButton::Primary);
        self.router.dispatch(&mut self.tree, &Event::Pointer(e));
        self.tata();
    }

    fn lepas(&mut self, titik: Point) {
        let t = self.maju(16);
        self.router.dispatch(
            &mut self.tree,
            &Event::Pointer(
                PointerEvent::new(PointerPhase::Up, titik, t).button(PointerButton::Primary),
            ),
        );
        self.tata();
    }

    /// Rapid-fire clicks with no pause: the router counts them as
    /// double/triple using the framework's own thresholds.
    fn klik_beruntun(&mut self, titik: Point, kali: u32) {
        for _ in 0..kali {
            let t = self.maju(60);
            for phase in [PointerPhase::Down, PointerPhase::Up] {
                self.router.dispatch(
                    &mut self.tree,
                    &Event::Pointer(
                        PointerEvent::new(phase, titik, t).button(PointerButton::Primary),
                    ),
                );
            }
        }
        self.tata();
    }

    /// One frame: advance every widget animation, then lay out — the same
    /// order the shell uses (`silka_platform::run_app_with`).
    fn bingkai_frame(&mut self, ms: u64) {
        let dt = Duration::from_millis(ms);
        self.jam += dt;
        let tick = Tick::manual(dt, Motion::Full);
        crate::advance(&mut self.tree, &tick);
        self.tree.layout(BoxConstraints::loose(RUANG));
    }

    /// What the frame cycle does after an event: sync, then lay out.
    ///
    /// It is `advance` and not a bare `layout` on purpose — that is exactly
    /// where the text area's height and its caret reveal are stitched back
    /// together (`text_area::sync`).
    fn tata(&mut self) {
        self.bingkai_frame(16);
    }

    fn scene(&mut self) -> Scene {
        let mut s = Scene::new(silka_paint::Color::BLACK);
        self.tree.paint_into(&mut s);
        s
    }

    fn glyph_runs(&mut self) -> usize {
        self.scene()
            .commands()
            .iter()
            .filter(|c| matches!(c, Command::GlyphRun(_)))
            .count()
    }
}

/// An `on_change` recorder.
#[derive(Default, Clone)]
struct Catatan(Rc<RefCell<Vec<String>>>);

impl Catatan {
    fn rekam(&self) -> impl Fn(&str) + 'static {
        let sisi = self.0.clone();
        move |s: &str| sisi.borrow_mut().push(s.to_string())
    }

    fn terakhir(&self) -> Option<String> {
        self.0.borrow().last().cloned()
    }

    fn jumlah(&self) -> usize {
        self.0.borrow().len()
    }
}

// ---------------------------------------------------------------------------
// Multi-line editing
// ---------------------------------------------------------------------------

#[test]
fn enter_menyisipkan_baris_baru_bukan_submit() {
    let f = fonts();
    let t = tema();
    let catatan = Catatan::default();
    let kirim = Catatan::default();
    let mut u = Uji::baru(
        text_area(&f, &t, "")
            .on_change(catatan.rekam())
            .on_submit(kirim.rekam()),
    );
    u.fokus();
    u.ketik("satu");
    u.nama(NamedKey::Enter, Modifiers::NONE);
    u.ketik("dua");

    assert_eq!(u.teks(), "satu\ndua");
    assert_eq!(catatan.terakhir().as_deref(), Some("satu\ndua"));
    assert_eq!(kirim.jumlah(), 0, "Enter polos bukan submit di text_area");

    // ⌘Enter is what submits — the comment-box habit.
    u.nama(NamedKey::Enter, Modifiers::COMMAND);
    assert_eq!(kirim.terakhir().as_deref(), Some("satu\ndua"));
    assert_eq!(u.teks(), "satu\ndua", "submit tidak menambah baris");
}

#[test]
fn panah_atas_bawah_mempertahankan_kolom_yang_diinginkan() {
    let f = fonts();
    let t = tema();
    // The middle line is deliberately the short one: this is exactly where a
    // naive implementation loses the column.
    let mut u = Uji::baru(text_area(&f, &t, "panjang sekali\nab\npanjang sekali"));
    u.fokus();
    u.nama(NamedKey::Home, Modifiers::COMMAND);
    for _ in 0..10 {
        u.nama(NamedKey::ArrowRight, Modifiers::NONE);
    }
    assert_eq!(u.badan().selection().focus, 10);

    // Down onto the short line: the caret can only reach its end…
    u.nama(NamedKey::ArrowDown, Modifiers::NONE);
    assert_eq!(u.badan().selection().focus, 17, "ujung baris pendek");

    // …and down again it comes back to the column the eye was on.
    u.nama(NamedKey::ArrowDown, Modifiers::NONE);
    assert_eq!(
        u.badan().selection().focus,
        28,
        "goal column hilang: caret tidak kembali ke kolom 10"
    );

    // A horizontal move drops the goal column, as in every real editor: the
    // caret comes back down to where it actually stands, not to column 10.
    u.nama(NamedKey::ArrowUp, Modifiers::NONE);
    assert_eq!(u.badan().selection().focus, 17);
    u.nama(NamedKey::ArrowLeft, Modifiers::NONE);
    u.nama(NamedKey::ArrowDown, Modifiers::NONE);
    let turun = u.badan().selection().focus;
    assert!(
        (18..22).contains(&turun),
        "gerak mendatar harus membuang goal column, tapi caret mendarat di {turun}"
    );
}

#[test]
fn panah_atas_di_baris_pertama_ke_awal_dokumen() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_area(&f, &t, EMPAT_BARIS));
    u.fokus();
    u.nama(NamedKey::Home, Modifiers::COMMAND);
    u.nama(NamedKey::ArrowRight, Modifiers::NONE);
    u.nama(NamedKey::ArrowUp, Modifiers::NONE);
    assert_eq!(u.badan().selection().focus, 0);

    u.nama(NamedKey::End, Modifiers::COMMAND);
    u.nama(NamedKey::ArrowDown, Modifiers::NONE);
    assert_eq!(u.badan().selection().focus, EMPAT_BARIS.len());
}

#[test]
fn home_end_bekerja_per_baris_dan_command_per_dokumen() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_area(&f, &t, EMPAT_BARIS));
    u.fokus();
    // Caret starts at the end of the document; End/Home stay on that line.
    u.nama(NamedKey::Home, Modifiers::NONE);
    assert_eq!(u.badan().selection().focus, 14, "awal baris \"empat\"");
    u.nama(NamedKey::End, Modifiers::NONE);
    assert_eq!(u.badan().selection().focus, EMPAT_BARIS.len());

    u.nama(NamedKey::Home, Modifiers::COMMAND);
    assert_eq!(u.badan().selection().focus, 0, "⌘Home = awal dokumen");
    u.nama(NamedKey::End, Modifiers::COMMAND);
    assert_eq!(u.badan().selection().focus, EMPAT_BARIS.len());
}

#[test]
fn seleksi_lintas_baris_menghasilkan_satu_kotak_per_baris() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_area(&f, &t, EMPAT_BARIS));
    u.fokus();
    u.nama(NamedKey::Home, Modifiers::COMMAND);
    for _ in 0..3 {
        u.nama(NamedKey::ArrowDown, Modifiers::SHIFT);
    }
    let seleksi = u.badan().selection();
    assert!(!seleksi.is_collapsed());
    assert!(
        u.badan().selection_rects().len() >= 3,
        "seleksi tiga baris harus punya kotak per baris: {:?}",
        u.badan().selection_rects()
    );
    // Each rectangle sits below the previous one.
    let kotak = u.badan().selection_rects().to_vec();
    for pasangan in kotak.windows(2) {
        assert!(pasangan[1].origin.y >= pasangan[0].origin.y);
    }
}

#[test]
fn klik_tiga_kali_memilih_satu_paragraf_bukan_seluruh_dokumen() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_area(&f, &t, EMPAT_BARIS));
    u.fokus();
    let baris = u.badan().lines()[1];
    let titik = Point::new(10.0, baris.top + baris.height / 2.0 + 8.0);
    u.klik_beruntun(titik, 3);
    let r = u.badan().selection().range();
    assert_eq!(&EMPAT_BARIS[r.clone()], "dua", "rentang {r:?}");
}

#[test]
fn seret_melintasi_baris_memperluas_seleksi() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_area(&f, &t, EMPAT_BARIS));
    let awal = Point::new(4.0, 10.0);
    u.tekan(awal);
    let bawah = u.badan().lines()[2];
    let akhir = Point::new(200.0, bawah.top + bawah.height / 2.0 + 8.0);
    u.seret(akhir);
    u.lepas(akhir);
    let s = u.badan().selection();
    assert!(!s.is_collapsed());
    assert!(s.end() >= 9, "seleksi harus mencapai baris ketiga: {s:?}");
}

// ---------------------------------------------------------------------------
// Soft wrap
// ---------------------------------------------------------------------------

#[test]
fn teks_panjang_terlipat_mengikuti_lebar_bukan_menggulir_ke_samping() {
    let f = fonts();
    let t = tema();
    let panjang = "kalimat yang sangat panjang sekali sampai harus dilipat beberapa kali";
    let mut u = Uji::baru(text_area(&f, &t, panjang));
    let baris = u.badan().lines().to_vec();
    assert!(baris.len() > 1, "teks selebar 320pt harus terlipat");
    // Every visual line belongs to the same **source** line: wrapping is not a
    // newline.
    assert!(baris.iter().all(|b| b.line == 0));
    assert!(
        u.teks().find('\n').is_none(),
        "melipat bukan menyisipkan \\n"
    );

    // Down/up still walk the wrapped lines one at a time.
    u.fokus();
    u.nama(NamedKey::Home, Modifiers::COMMAND);
    u.nama(NamedKey::ArrowDown, Modifiers::NONE);
    let fokus = u.badan().selection().focus;
    assert!(fokus > 0 && fokus < panjang.len());
    assert_eq!(
        fokus, baris[1].start,
        "panah bawah harus mendarat di baris visual kedua"
    );
}

#[test]
fn home_di_baris_terlipat_ke_awal_baris_visual() {
    let f = fonts();
    let t = tema();
    let panjang = "kalimat yang sangat panjang sekali sampai harus dilipat beberapa kali";
    let mut u = Uji::baru(text_area(&f, &t, panjang));
    let baris = u.badan().lines().to_vec();
    u.fokus();
    // The caret starts at the end of the document, i.e. on the last visual
    // line; Home there means the start of *that* line, not of the paragraph.
    u.nama(NamedKey::Home, Modifiers::NONE);
    assert_eq!(u.badan().selection().focus, baris[baris.len() - 1].start);
    assert_ne!(u.badan().selection().focus, 0);
}

// ---------------------------------------------------------------------------
// Tab
// ---------------------------------------------------------------------------

#[test]
fn tab_secara_bawaan_memindahkan_fokus_bukan_menyisipkan() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(silka_core::view::column([
        View::from(text_area(&f, &t, "").key("a")),
        View::from(text_area(&f, &t, "").key("b")),
    ]));
    let kedua = bodies(&u.tree)[1];
    u.fokus();

    u.nama(NamedKey::Tab, Modifiers::NONE);
    assert_eq!(
        u.teks(),
        "",
        "Tab yang ditelan kolom teks = jebakan keyboard (DoD aksesibilitas)"
    );
    assert_eq!(
        u.router.focus().focused(),
        Some(kedua),
        "Tab harus diserahkan ke navigasi fokus"
    );
}

#[test]
fn tab_bisa_dikonfigurasi_untuk_menyisipkan_indentasi() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_area(&f, &t, "").tab(TabBehavior::InsertTab));
    u.fokus();
    let r = u.nama(NamedKey::Tab, Modifiers::NONE);
    assert_eq!(u.teks(), "\t");
    assert!(!r.focus.changed(), "indentasi tidak memindahkan fokus");

    // ⇧Tab still walks focus backwards: an escape hatch always exists, even
    // when Tab itself indents.
    u.nama(NamedKey::Tab, Modifiers::SHIFT);
    assert_eq!(u.teks(), "\t", "⇧Tab bukan indentasi");
}

#[test]
fn fokus_bisa_keluar_lewat_tab_ke_widget_berikutnya() {
    let f = fonts();
    let t = tema();
    let mut tree = RenderTree::new();
    reconcile(
        &mut tree,
        silka_core::view::column([
            View::from(text_area(&f, &t, "").key("a")),
            View::from(text_area(&f, &t, "").key("b")),
        ]),
    );
    tree.layout(BoxConstraints::loose(Size::new(320.0, 600.0)));
    let mut router = InputRouter::new();
    let badan = bodies(&tree);
    assert_eq!(badan.len(), 2);

    router.focus_node(&mut tree, Some(badan[0]));
    router.move_focus(&mut tree, FocusDirection::Next);
    assert_eq!(
        router.focus().focused(),
        Some(badan[1]),
        "text_area harus jadi satu perhentian Tab, dan bisa ditinggalkan"
    );
}

// ---------------------------------------------------------------------------
// Height: fixed and auto-grow
// ---------------------------------------------------------------------------

#[test]
fn tinggi_tetap_tidak_ikut_tumbuh_saat_isi_bertambah() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_area(&f, &t, "").rows(3));
    let awal = u.tinggi();
    u.fokus();
    for _ in 0..10 {
        u.nama(NamedKey::Enter, Modifiers::NONE);
    }
    assert_eq!(u.tinggi(), awal, "rows() adalah tinggi tetap");
    // The content is taller than the box, so the scroll view really can scroll.
    assert!(u.gulir().can_scroll());
}

#[test]
fn auto_grow_tumbuh_bersama_isi_lalu_berhenti_di_batas() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_area(&f, &t, "").auto_grow(2, 5));
    let kecil = u.tinggi();
    u.fokus();

    u.nama(NamedKey::Enter, Modifiers::NONE);
    u.nama(NamedKey::Enter, Modifiers::NONE);
    let sedang = u.tinggi();
    assert!(sedang > kecil, "kotak harus tumbuh: {kecil} -> {sedang}");

    // Past the maximum it stops growing and starts scrolling instead.
    for _ in 0..10 {
        u.nama(NamedKey::Enter, Modifiers::NONE);
    }
    let besar = u.tinggi();
    assert!(besar > sedang);
    assert!(
        besar <= u.badan().lines().len() as f32 * 100.0,
        "sanity: tinggi tidak meledak"
    );
    let (_, maks) = text_area(&f, &t, "").auto_grow(2, 5).height_range();
    assert!(
        (besar - maks).abs() < 0.5,
        "tinggi {besar} harus berhenti di batas {maks}"
    );
    assert!(u.gulir().can_scroll());
}

#[test]
fn auto_grow_menyusut_lagi_saat_isi_dihapus() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_area(&f, &t, "satu\ndua\ntiga\nempat\nlima").auto_grow(1, 8));
    let besar = u.tinggi();
    u.fokus();
    u.tombol(KeyCode::Character('a'), Modifiers::COMMAND);
    u.nama(NamedKey::Backspace, Modifiers::NONE);
    assert_eq!(u.teks(), "");
    assert!(
        u.tinggi() < besar,
        "kotak harus menyusut kembali: {besar} -> {}",
        u.tinggi()
    );
}

// ---------------------------------------------------------------------------
// Scrolling the caret into view
// ---------------------------------------------------------------------------

#[test]
fn caret_yang_keluar_layar_menarik_gulirannya_kembali() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_area(&f, &t, "").rows(3));
    u.fokus();
    for i in 0..20 {
        u.ketik(&format!("baris{i}"));
        u.nama(NamedKey::Enter, Modifiers::NONE);
    }
    // The sync pass runs inside the frame cycle, not inside the event.
    for _ in 0..40 {
        u.bingkai_frame(16);
    }
    assert!(
        u.gulir().offset() > 0.0,
        "caret di baris ke-20 harus menggulirkan isinya"
    );

    // Back to the top of the document, and the scroll follows.
    u.nama(NamedKey::Home, Modifiers::COMMAND);
    for _ in 0..400 {
        u.bingkai_frame(16);
        if !crate::is_animating(&u.tree) {
            break;
        }
    }
    // Back at the very top: what is left is the few points of padding the
    // reveal deliberately keeps around the caret.
    assert!(
        u.gulir().offset() < 8.0,
        "caret kembali ke awal harus menggulirkan balik: {}",
        u.gulir().offset()
    );
}

// ---------------------------------------------------------------------------
// Gutter and placeholder
// ---------------------------------------------------------------------------

#[test]
fn nomor_baris_menyisakan_ruang_dan_menggambar_satu_angka_per_baris_sumber() {
    let f = fonts();
    let t = tema();
    let tanpa = Uji::baru(text_area(&f, &t, EMPAT_BARIS));
    let mut dengan = Uji::baru(text_area(&f, &t, EMPAT_BARIS).line_numbers(true));
    assert_eq!(tanpa.badan().gutter_width(), 0.0);
    assert!(dengan.badan().gutter_width() > 0.0);

    // One run for the text plus one per source line.
    assert_eq!(dengan.glyph_runs(), 5, "empat nomor baris + satu run teks");
}

#[test]
fn baris_terlipat_tidak_mendapat_nomor_sendiri() {
    let f = fonts();
    let t = tema();
    let panjang = "kalimat yang sangat panjang sekali sampai harus dilipat beberapa kali";
    let mut u = Uji::baru(text_area(&f, &t, panjang).line_numbers(true));
    assert!(u.badan().lines().len() > 1, "harus terlipat");
    assert_eq!(
        u.glyph_runs(),
        2,
        "satu paragraf = satu nomor, walau terlipat jadi beberapa baris"
    );
}

#[test]
fn placeholder_tampil_saat_kosong_dan_hilang_saat_diketik() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_area(&f, &t, "").placeholder("Tulis catatan"));
    assert!(u.badan().shows_placeholder());
    u.fokus();
    u.ketik("a");
    assert!(!u.badan().shows_placeholder());
    u.nama(NamedKey::Backspace, Modifiers::NONE);
    assert!(u.badan().shows_placeholder());
}

// ---------------------------------------------------------------------------
// IME
// ---------------------------------------------------------------------------

#[test]
fn preedit_ime_terlihat_tapi_belum_sampai_ke_aplikasi() {
    let f = fonts();
    let t = tema();
    let catatan = Catatan::default();
    let mut u = Uji::baru(text_area(&f, &t, "").on_change(catatan.rekam()));
    u.fokus();
    u.ime(ImeEvent::Enabled);
    u.ime(ImeEvent::Preedit {
        text: "にほn".into(),
        cursor: None,
    });
    assert!(u.badan().is_composing());
    assert_eq!(u.teks(), "", "komposisi belum jadi isi");
    assert_eq!(
        catatan.jumlah(),
        0,
        "aplikasi tidak menerima huruf setengah jadi"
    );
    assert!(
        !u.badan().preedit_rects().is_empty(),
        "preedit digarisbawahi"
    );

    // The normal key path is held back during composition.
    u.nama(NamedKey::Enter, Modifiers::NONE);
    assert_eq!(u.teks(), "", "Enter selama komposisi bukan baris baru");

    u.ime(ImeEvent::Commit("日本".into()));
    assert_eq!(u.teks(), "日本");
    assert_eq!(catatan.terakhir().as_deref(), Some("日本"));
}

// ---------------------------------------------------------------------------
// Disabled / read-only
// ---------------------------------------------------------------------------

#[test]
fn read_only_boleh_diseleksi_tapi_tidak_bisa_diubah() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_area(&f, &t, EMPAT_BARIS).read_only(true));
    u.fokus();
    u.ketik("x");
    u.nama(NamedKey::Enter, Modifiers::NONE);
    u.nama(NamedKey::Backspace, Modifiers::NONE);
    assert_eq!(u.teks(), EMPAT_BARIS);

    u.tombol(KeyCode::Character('a'), Modifiers::COMMAND);
    assert!(!u.badan().selection().is_collapsed(), "seleksi tetap boleh");
}

#[test]
fn kolom_mati_tidak_menerima_fokus_tapi_tetap_dibacakan() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_area(&f, &t, "isi").disabled(true).label("Catatan"));
    u.fokus();
    u.ketik("x");
    assert_eq!(u.teks(), "isi");

    let pohon = u.tree.access_tree(None);
    let e = pohon.find_label("Catatan").expect("tetap dibacakan");
    assert!(e.node.disabled);
    assert!(!e.node.actions.contains(AccessActions::FOCUS));
}

// ---------------------------------------------------------------------------
// Accessibility
// ---------------------------------------------------------------------------

#[test]
fn node_a11y_berperan_multiline_dan_melaporkan_caret() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_area(&f, &t, EMPAT_BARIS).label("Catatan"));
    u.fokus();
    u.nama(NamedKey::Home, Modifiers::COMMAND);
    for _ in 0..2 {
        u.nama(NamedKey::ArrowRight, Modifiers::SHIFT);
    }

    let pohon = u.tree.access_tree(Some(u.id));
    let e = pohon.find_label("Catatan").expect("kolom harus dibacakan");
    assert_eq!(e.node.role, AccessRole::MultilineTextInput);
    assert_eq!(e.node.value.as_deref(), Some(EMPAT_BARIS));
    assert!(e.node.actions.contains(AccessActions::SET_VALUE));
    let s = e.node.text_selection.expect("caret harus dilaporkan");
    assert_eq!(s.anchor, 0);
    assert_eq!(s.focus, 2, "seleksi dua karakter pertama");

    // The scroll container around it advertises SCROLL on its own.
    let gulir = pohon
        .entries()
        .iter()
        .find(|e| e.node.role == AccessRole::ScrollView);
    assert!(gulir.is_some(), "wadah guliran harus ikut dibacakan");
}

#[test]
fn caret_dilaporkan_dalam_karakter_bukan_byte() {
    let f = fonts();
    let t = tema();
    // "日本" is two characters but six bytes: a screen reader counts the
    // former.
    let mut u = Uji::baru(text_area(&f, &t, "日本").label("Catatan"));
    u.fokus();
    let pohon = u.tree.access_tree(Some(u.id));
    let s = pohon
        .find_label("Catatan")
        .and_then(|e| e.node.text_selection)
        .expect("caret");
    assert_eq!(s.focus, 2, "dua karakter, bukan enam byte");
}

#[test]
fn teknologi_bantu_bisa_mengganti_isi_lewat_set_value() {
    use silka_core::access::{AccessAction, AccessActionRequest};

    let f = fonts();
    let t = tema();
    let catatan = Catatan::default();
    let mut u = Uji::baru(text_area(&f, &t, "lama").on_change(catatan.rekam()));
    let permintaan = AccessActionRequest {
        target: u.id,
        action: AccessAction::SetValue,
        value: Some("didikte".into()),
    };
    assert!(apply_access_action(&mut u.tree, &permintaan));
    assert_eq!(u.teks(), "didikte");
    assert_eq!(catatan.terakhir().as_deref(), Some("didikte"));
}

// ---------------------------------------------------------------------------
// Tokens, presets, motion
// ---------------------------------------------------------------------------

#[test]
fn latar_dan_sudut_datang_dari_token_di_kedua_preset() {
    let f = fonts();
    for preset in [Preset::Cupertino, Preset::Tailwind] {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let t = Theme::new(preset, appearance);
            let mut u = Uji::baru(text_area(&f, &t, "isi"));
            let s = u.scene();
            let quad = s
                .commands()
                .iter()
                .find_map(|c| match c {
                    Command::Quad(q) => Some(q.clone()),
                    _ => None,
                })
                .expect("bingkai harus menggambar latar");
            assert_eq!(quad.background, t.color.surface);
            assert_eq!(
                quad.corners.style, t.radius.style,
                "bentuk sudut mengikuti preset, bukan konstanta"
            );
        }
    }
}

#[test]
fn cincin_fokus_tumbuh_lewat_spring_lalu_pohon_kembali_diam() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_area(&f, &t, "isi"));
    u.fokus();

    let bingkai = u.bingkai();
    let awal = u
        .tree
        .node_ref::<TextAreaFrame>(bingkai)
        .unwrap()
        .focus_progress();
    assert_eq!(awal, 0.0, "cincin mulai dari nol, tidak muncul mendadak");

    u.bingkai_frame(16);
    let sesudah = u
        .tree
        .node_ref::<TextAreaFrame>(bingkai)
        .unwrap()
        .focus_progress();
    assert!(
        sesudah > 0.0 && sesudah < 1.0,
        "cincin harus bergerak: {sesudah}"
    );

    for _ in 0..400 {
        u.bingkai_frame(8);
        if !is_animating(&u.tree) {
            break;
        }
    }
    assert!(!is_animating(&u.tree), "transisi tidak pernah settle");
    assert!(
        u.tree
            .node_ref::<TextAreaFrame>(bingkai)
            .unwrap()
            .focus_progress()
            > 0.99
    );
}

#[test]
fn reduced_motion_sampai_ke_tujuan_tanpa_memantul() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_area(&f, &t, "isi"));
    u.fokus();
    let bingkai = u.bingkai();
    let mut tertinggi: f32 = 0.0;
    for _ in 0..400 {
        let tick = Tick::manual(Duration::from_millis(16), Motion::Reduced);
        crate::advance(&mut u.tree, &tick);
        let p = u
            .tree
            .node_ref::<TextAreaFrame>(bingkai)
            .unwrap()
            .focus_progress();
        tertinggi = tertinggi.max(p);
        if !is_animating(&u.tree) {
            break;
        }
    }
    assert!(!is_animating(&u.tree), "transisi tidak pernah settle");
    assert!(
        tertinggi <= 1.0 + f32::EPSILON,
        "reduced-motion membuang pantulan, tapi cincin melewati tujuannya: {tertinggi}"
    );
    assert!(
        u.tree
            .node_ref::<TextAreaFrame>(bingkai)
            .unwrap()
            .focus_progress()
            > 0.99,
        "cincin tetap sampai — reduced-motion menghapus perjalanannya, bukan tujuannya"
    );
}

#[test]
fn settle_menyelesaikan_semua_transisi_sekaligus() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_area(&f, &t, "isi"));
    u.fokus();
    crate::settle(&mut u.tree);
    assert!(!is_animating(&u.tree));
}

// ---------------------------------------------------------------------------
// The controlled-component rule
// ---------------------------------------------------------------------------

#[test]
fn rebuild_dengan_nilai_props_sama_tidak_melempar_caret_ke_belakang() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_area(&f, &t, ""));
    u.fokus();
    u.ketik("halo");
    u.nama(NamedKey::Home, Modifiers::NONE);

    // The app rebuilds for an unrelated reason: the props still say "" because
    // this area is uncontrolled.
    reconcile(&mut u.tree, View::from(text_area(&f, &t, "")));
    u.tata();
    assert_eq!(u.teks(), "halo", "isi tidak boleh ditimpa props yang sama");
    assert_eq!(u.badan().selection().focus, 0, "caret tidak dilempar");
    assert!(
        u.tree
            .node_ref::<TextAreaFrame>(u.bingkai())
            .is_some_and(|b| b.focus_progress() >= 0.0),
        "bingkai bertahan lintas rebuild"
    );
}

#[test]
fn nilai_baru_dari_aplikasi_tetap_menimpa_isi() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_area(&f, &t, "lama"));
    reconcile(&mut u.tree, View::from(text_area(&f, &t, "baru\nsekali")));
    u.tata();
    assert_eq!(u.teks(), "baru\nsekali");
    assert_eq!(u.badan().lines().len(), 2);
}
