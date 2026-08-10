//! `text_field` tests — every one of them through the **input layer**, never by
//! calling internal methods.
//!
//! The reasoning is the same as for the `button` tests clicking via coordinates
//! taken from the accessibility tree: what has to be proven is not "this
//! function returns that value" but "a user who types, clicks, or builds up an
//! IME composition gets the right result". Everything purely Unicode
//! (graphemes, words, undo, preedit) is already tested in `silka_text::edit`;
//! what is tested here is the wiring into the tree, the geometry, the tokens,
//! a11y, and the springs.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use silka_core::access::{AccessActions, AccessRole};
use silka_core::animation::{Motion, Tick};
use silka_core::input::{
    Event, ImeEvent, ImeRequest, InputRouter, KeyCode, KeyEvent, Modifiers, NamedKey,
    PointerButton, PointerEvent, PointerPhase, Response,
};
use silka_core::scheduler::Dirty;
use silka_core::tree::{BoxConstraints, NodeId, RenderTree};
use silka_core::view::{reconcile, View};
use silka_paint::{Command, Point, Rect, Scene, Size};
use silka_theme::{Appearance, Preset, Theme};

use super::*;
use crate::fonts::Fonts;

const RUANG: Size = Size::new(320.0, 200.0);

/// ZWJ family emoji: one grapheme, 25 bytes.
const KELUARGA: &str = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";

/// A single text field inside a tree, wired up to an input router.
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
        let id = first(&tree).expect("kolom teks harus ada di pohon");
        Self {
            tree,
            router: InputRouter::new(),
            id,
            jam: Duration::ZERO,
        }
    }

    fn kolom(&self) -> &TextFieldBox {
        self.tree.node_ref::<TextFieldBox>(self.id).expect("kolom")
    }

    fn teks(&self) -> String {
        self.kolom().text().to_string()
    }

    fn ukuran(&self) -> Size {
        self.tree.size(self.id)
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
        self.router.dispatch(&mut self.tree, &Event::Key(e))
    }

    fn ketik(&mut self, teks: &str) {
        for c in teks.chars() {
            self.tombol(KeyCode::Character(c), Modifiers::NONE);
        }
    }

    fn ime(&mut self, e: ImeEvent) -> Response {
        self.router.dispatch(&mut self.tree, &Event::Ime(e))
    }

    /// Press at `x` (local == global coordinates: the field sits at the tree's
    /// corner).
    fn tekan(&mut self, x: f32) {
        let y = self.ukuran().height / 2.0;
        let t = self.maju(300);
        self.router.dispatch(
            &mut self.tree,
            &Event::Pointer(
                PointerEvent::new(PointerPhase::Down, Point::new(x, y), t)
                    .button(PointerButton::Primary),
            ),
        );
    }

    fn seret(&mut self, x: f32) {
        let y = self.ukuran().height / 2.0;
        let t = self.maju(16);
        let mut e = PointerEvent::new(PointerPhase::Move, Point::new(x, y), t);
        e.buttons.insert(PointerButton::Primary);
        self.router.dispatch(&mut self.tree, &Event::Pointer(e));
    }

    fn lepas(&mut self, x: f32) {
        let y = self.ukuran().height / 2.0;
        let t = self.maju(16);
        self.router.dispatch(
            &mut self.tree,
            &Event::Pointer(
                PointerEvent::new(PointerPhase::Up, Point::new(x, y), t)
                    .button(PointerButton::Primary),
            ),
        );
    }

    fn klik(&mut self, x: f32) {
        self.tekan(x);
        self.lepas(x);
    }

    /// Rapid-fire clicks **with no pause**: the router is what counts them as
    /// double/triple, using the framework's own thresholds — not a number
    /// written down here.
    fn klik_beruntun(&mut self, x: f32, kali: u32) {
        let y = self.ukuran().height / 2.0;
        for _ in 0..kali {
            let t = self.maju(60);
            for phase in [PointerPhase::Down, PointerPhase::Up] {
                self.router.dispatch(
                    &mut self.tree,
                    &Event::Pointer(
                        PointerEvent::new(phase, Point::new(x, y), t)
                            .button(PointerButton::Primary),
                    ),
                );
            }
        }
    }

    fn scene(&mut self) -> Scene {
        let mut s = Scene::new(silka_paint::Color::BLACK);
        self.tree.paint_into(&mut s);
        s
    }

    fn glyph_count(&mut self) -> usize {
        self.scene()
            .commands()
            .iter()
            .filter_map(|c| match c {
                Command::GlyphRun(r) => Some(r.len()),
                _ => None,
            })
            .sum()
    }

    /// Re-layout the way the frame cycle does once something is marked dirty.
    fn tata(&mut self) {
        self.tree.layout(BoxConstraints::loose(RUANG));
    }
}

fn fonts() -> Fonts {
    Fonts::bundled_only()
}

fn tema() -> Theme {
    Theme::cupertino(Appearance::Dark)
}

/// An `on_change` recorder: how many times it fired, and with what.
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
// Shape & accessibility
// ---------------------------------------------------------------------------

#[test]
fn hit_target_minimal_44pt_walau_barisnya_pendek() {
    let f = fonts();
    let t = tema();
    let u = Uji::baru(text_field(&f, &t, ""));
    let ukuran = u.ukuran();
    assert!(
        ukuran.height >= MIN_HIT_TARGET,
        "hit target cuma {ukuran:?} (HIG minta {MIN_HIT_TARGET}pt)"
    );
    assert_eq!(
        ukuran.width, RUANG.width,
        "kolom mengisi lebar yang tersedia"
    );
}

#[test]
fn dibacakan_screen_reader_sebagai_kolom_teks_berisi_nilainya() {
    let f = fonts();
    let t = tema();
    let u = Uji::baru(text_field(&f, &t, "Ubud").label("Kota"));
    let pohon = u.tree.access_tree(None);
    let e = pohon
        .find_label("Kota")
        .unwrap_or_else(|| panic!("{}", pohon.dump()));

    assert_eq!(e.node.role, AccessRole::TextInput);
    assert_eq!(e.node.value.as_deref(), Some("Ubud"));
    assert!(e.node.actions.contains(AccessActions::FOCUS));
    assert!(e.node.actions.contains(AccessActions::CLICK));
    assert!(
        e.node.actions.contains(AccessActions::SET_VALUE),
        "dikte suara harus bisa mengisi kolom"
    );
    // The bounds come from the layout result, not from the widget.
    assert_eq!(e.bounds.size, u.ukuran());
}

#[test]
fn kolom_mati_dibacakan_tapi_tidak_bisa_dipakai() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, "beku").label("Kunci").disabled(true));
    let pohon = u.tree.access_tree(None);
    let e = pohon.find_label("Kunci").expect("tetap dibacakan");
    assert!(e.node.disabled);
    assert!(!e.node.actions.contains(AccessActions::FOCUS));
    assert!(!e.node.actions.contains(AccessActions::SET_VALUE));

    // Clicking grants no focus, and keystrokes go nowhere.
    u.klik(20.0);
    u.ketik("x");
    assert_eq!(u.teks(), "beku");
    assert!(u.router.focus().focused().is_none());
}

#[test]
fn placeholder_tampil_saat_kosong_lalu_menghilang_saat_diketik() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, "").placeholder("Cari…"));
    assert!(u.kolom().shows_placeholder());
    let kosong = u.glyph_count();
    assert!(kosong > 0, "placeholder harus benar-benar tergambar");

    u.fokus();
    u.ketik("a");
    u.tata();
    assert!(!u.kolom().shows_placeholder());
    assert_eq!(u.teks(), "a");
}

// ---------------------------------------------------------------------------
// Typing
// ---------------------------------------------------------------------------

#[test]
fn mengetik_lewat_lapisan_input_mengisi_kolom_dan_melapor_sekali_per_huruf() {
    let f = fonts();
    let t = tema();
    let catatan = Catatan::default();
    let mut u = Uji::baru(text_field(&f, &t, "").on_change(catatan.rekam()));
    u.fokus();
    u.ketik("halo");
    assert_eq!(u.teks(), "halo");
    assert_eq!(catatan.jumlah(), 4);
    assert_eq!(catatan.terakhir().as_deref(), Some("halo"));

    // Space is a named key, not a character — and it still produces text.
    u.tombol(KeyCode::Named(NamedKey::Space), Modifiers::NONE);
    assert_eq!(u.teks(), "halo ");
}

#[test]
fn tanpa_fokus_ketikan_tidak_masuk_ke_mana_pun() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, ""));
    u.ketik("hantu");
    assert_eq!(u.teks(), "");
}

#[test]
fn backspace_menghapus_satu_grapheme_utuh_bukan_satu_byte() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, format!("a{KELUARGA}")));
    u.fokus();
    u.tombol(KeyCode::Named(NamedKey::Backspace), Modifiers::NONE);
    assert_eq!(u.teks(), "a", "emoji ZWJ tidak boleh terbelah");
}

#[test]
fn pintasan_pilih_semua_lalu_ketik_mengganti_seluruh_isi() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, "lama"));
    u.fokus();
    u.tombol(KeyCode::Character('a'), Modifiers::COMMAND);
    assert_eq!(u.kolom().selection().range(), 0..4);
    u.ketik("baru");
    assert_eq!(u.teks(), "baru");
}

#[test]
fn undo_dan_redo_lewat_pintasan() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, ""));
    u.fokus();
    u.ketik("draf");
    u.tombol(KeyCode::Character('z'), Modifiers::COMMAND);
    assert_eq!(u.teks(), "", "satu kata yang diketik = satu langkah undo");
    u.tombol(
        KeyCode::Character('z'),
        Modifiers::COMMAND | Modifiers::SHIFT,
    );
    assert_eq!(u.teks(), "draf");
}

#[test]
fn enter_memanggil_on_submit_dengan_isi_kolom() {
    let f = fonts();
    let t = tema();
    let catatan = Catatan::default();
    let mut u = Uji::baru(text_field(&f, &t, "kirim").on_submit(catatan.rekam()));
    u.fokus();
    u.tombol(KeyCode::Named(NamedKey::Enter), Modifiers::NONE);
    assert_eq!(catatan.terakhir().as_deref(), Some("kirim"));
}

#[test]
fn kolom_read_only_bisa_diseleksi_tapi_tidak_bisa_diubah() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, "tetap").read_only(true));
    u.fokus();
    u.ketik("x");
    u.tombol(KeyCode::Named(NamedKey::Backspace), Modifiers::NONE);
    assert_eq!(u.teks(), "tetap");
    u.tombol(KeyCode::Character('a'), Modifiers::COMMAND);
    assert_eq!(u.kolom().selection().range(), 0..5);

    let pohon = u.tree.access_tree(None);
    let e = &pohon.entries()[pohon.entries().len() - 1];
    assert!(!e.node.actions.contains(AccessActions::SET_VALUE));
}

#[test]
fn tab_tidak_ditelan_kolom_teks() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, ""));
    u.fokus();
    let r = u.tombol(KeyCode::Named(NamedKey::Tab), Modifiers::NONE);
    assert_eq!(u.teks(), "", "Tab bukan karakter");
    // The router is what uses it for focus navigation — and rightly so.
    assert!(r.handled);
}

#[test]
fn escape_dibiarkan_menggelembung_ke_overlay() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, "isi"));
    u.fokus();
    let r = u.tombol(KeyCode::Named(NamedKey::Escape), Modifiers::NONE);
    assert!(!r.handled, "Esc milik dialog/popover, bukan kolom teks");
}

// ---------------------------------------------------------------------------
// Caret, selection, and pointer
// ---------------------------------------------------------------------------

#[test]
fn klik_menaruh_caret_di_tempat_yang_diklik() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, "satu dua tiga"));
    u.fokus();

    // Clicking far to the right of the text = caret at the end.
    u.klik(RUANG.width - 4.0);
    assert_eq!(u.kolom().selection().range(), 13..13);

    // Clicking at the start of the text = caret at the beginning.
    u.klik(1.0);
    assert_eq!(u.kolom().selection().range(), 0..0);
    assert!(u.kolom().selection().is_collapsed());
}

#[test]
fn klik_ganda_menyeleksi_kata_klik_tripel_seluruh_isi() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, "satu dua tiga"));
    u.fokus();

    // A point in the **middle** of the word "dua" (indices 5..8): derived from
    // two real caret positions, not from a guessed number.
    u.klik(1.0);
    let caret_x = |u: &mut Uji, n: usize| {
        u.tombol(KeyCode::Named(NamedKey::Home), Modifiers::NONE);
        for _ in 0..n {
            u.tombol(KeyCode::Named(NamedKey::ArrowRight), Modifiers::NONE);
        }
        u.kolom().caret_rect().origin.x
    };
    let x = (caret_x(&mut u, 5) + caret_x(&mut u, 8)) / 2.0;

    u.klik_beruntun(x, 2);
    assert_eq!(
        u.kolom().selection().range(),
        5..8,
        "klik ganda = satu kata"
    );

    u.klik_beruntun(x, 3);
    assert_eq!(
        u.kolom().selection().range(),
        0..13,
        "klik tripel = semuanya"
    );
}

#[test]
fn seret_menyeleksi_rentang_dan_kotak_sorotnya_digambar() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, "satu dua tiga"));
    u.fokus();

    u.tekan(1.0);
    u.seret(60.0);
    let seleksi = u.kolom().selection();
    assert!(!seleksi.is_collapsed(), "menyeret harus menyeleksi");
    assert_eq!(seleksi.anchor, 0);
    assert!(!u.kolom().selection_rects().is_empty());
    u.lepas(60.0);
    // The highlight fades along with the focus transition; the transition is
    // settled first so what gets compared is the token color, not a color
    // caught mid-animation.
    crate::settle(&mut u.tree);

    // The highlight really does reach the draw commands, underneath the text.
    let scene = u.scene();
    let kuas: Vec<_> = scene
        .commands()
        .iter()
        .filter_map(|c| match c {
            Command::Quad(q) if q.background == t.color.selection => Some(q.rect),
            _ => None,
        })
        .collect();
    assert!(!kuas.is_empty(), "kotak seleksi tidak digambar");
    assert!(kuas[0].size.width > 0.0);
}

#[test]
fn shift_klik_memperluas_seleksi_dari_caret_yang_ada() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, "satu dua"));
    u.fokus();
    u.klik(1.0);

    let y = u.ukuran().height / 2.0;
    let t_klik = u.maju(300);
    u.router.dispatch(
        &mut u.tree,
        &Event::Pointer(
            PointerEvent::new(PointerPhase::Down, Point::new(RUANG.width - 4.0, y), t_klik)
                .button(PointerButton::Primary)
                .modifiers(Modifiers::SHIFT),
        ),
    );
    assert_eq!(u.kolom().selection().range(), 0..8);
}

#[test]
fn caret_hanya_digambar_saat_kolom_terfokus() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, "isi"));
    let caret = |u: &mut Uji| -> usize {
        let kotak = u.kolom().caret_rect();
        u.scene()
            .commands()
            .iter()
            .filter(|c| matches!(c, Command::Quad(q) if q.rect == kotak))
            .count()
    };
    assert_eq!(caret(&mut u), 0, "kolom diam tidak punya caret");
    u.fokus();
    assert_eq!(caret(&mut u), 1);
}

#[test]
fn caret_selalu_terlihat_walau_isinya_lebih_panjang_dari_kolom() {
    let f = fonts();
    let t = tema();
    let panjang = "kalimat yang jauh lebih panjang daripada lebar kolomnya sendiri, sungguh";
    let mut u = Uji::baru(text_field(&f, &t, panjang));
    u.fokus();
    // Caret to the far right: the contents must scroll, not the caret vanish.
    u.tombol(KeyCode::Named(NamedKey::End), Modifiers::NONE);
    let kotak = u.kolom().caret_rect();
    assert!(u.kolom().scroll() > 0.0, "isi harus tergulir");
    assert!(
        kotak.min_x() >= 0.0 && kotak.max_x() <= u.ukuran().width,
        "caret keluar kolom: {kotak:?}"
    );

    // Back to the start: the scroll comes home, the caret sits at the front.
    u.tombol(KeyCode::Named(NamedKey::Home), Modifiers::NONE);
    assert_eq!(u.kolom().scroll(), 0.0);
}

#[test]
fn glyph_dipotong_di_tepi_kolom_bukan_menabrak_border() {
    let f = fonts();
    let t = tema();
    let panjang = "kalimat yang jauh lebih panjang daripada lebar kolomnya sendiri, sungguh";
    let mut u = Uji::baru(text_field(&f, &t, panjang));
    let scene = u.scene();
    let run = scene
        .commands()
        .iter()
        .find_map(|c| match c {
            Command::GlyphRun(r) => Some(r.clone()),
            _ => None,
        })
        .expect("teks tergambar");
    let clip = run.clip.expect("run kolom teks selalu dipotong");
    assert!(
        clip.size.width < RUANG.width,
        "clip = kotak isi, bukan kotak node"
    );
    assert!(clip.min_x() > 0.0);
}

// ---------------------------------------------------------------------------
// IME
// ---------------------------------------------------------------------------

#[test]
fn preedit_dirender_inline_tapi_tidak_pernah_dilaporkan_ke_aplikasi() {
    let f = fonts();
    let t = tema();
    let catatan = Catatan::default();
    let mut u = Uji::baru(text_field(&f, &t, "ha").on_change(catatan.rekam()));
    u.fokus();
    u.tombol(KeyCode::Named(NamedKey::End), Modifiers::NONE);
    let sebelum = u.glyph_count();

    u.ime(ImeEvent::Enabled);
    u.ime(ImeEvent::Preedit {
        text: "ni".into(),
        cursor: Some((2, 2)),
    });
    u.tata();

    assert!(u.kolom().is_composing());
    assert_eq!(u.teks(), "ha", "preedit belum jadi isi kolom");
    assert_eq!(
        catatan.jumlah(),
        0,
        "aplikasi tidak boleh melihat huruf setengah jadi"
    );
    assert!(u.glyph_count() > sebelum, "preedit harus terlihat inline");
    assert!(
        !u.kolom().preedit_rects().is_empty(),
        "preedit wajib bergaris bawah (§3.8)"
    );

    // Commit turns it into real contents, reported exactly once.
    u.ime(ImeEvent::Commit("に".into()));
    u.tata();
    assert!(!u.kolom().is_composing());
    assert_eq!(
        u.teks(),
        "haに",
        "commit membuang preedit, bukan menempelinya"
    );
    assert_eq!(catatan.terakhir().as_deref(), Some("haに"));
    assert!(u.kolom().preedit_rects().is_empty());
}

#[test]
fn selama_komposisi_jalur_tombol_normal_ditahan() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, ""));
    u.fokus();
    u.ime(ImeEvent::Preedit {
        text: "ni".into(),
        cursor: None,
    });

    // Letters and arrows arriving mid-composition belong to the IME, not us.
    u.ketik("x");
    u.tombol(KeyCode::Named(NamedKey::Backspace), Modifiers::NONE);
    assert_eq!(u.teks(), "");
    assert!(u.kolom().is_composing());
}

#[test]
fn komposisi_dibatalkan_saat_ime_dimatikan_atau_fokus_pergi() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, "x"));
    u.fokus();
    u.ime(ImeEvent::Preedit {
        text: "ni".into(),
        cursor: None,
    });
    u.ime(ImeEvent::Disabled);
    assert!(!u.kolom().is_composing());

    u.ime(ImeEvent::Preedit {
        text: "ni".into(),
        cursor: None,
    });
    u.router.focus_node(&mut u.tree, None);
    assert!(!u.kolom().is_composing(), "komposisi menggantung dibuang");
    assert_eq!(u.teks(), "x");
}

#[test]
fn area_kandidat_ime_mengikuti_caret() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, "halo dunia"));
    let r = u.fokus();
    let area = match r.ime {
        Some(ImeRequest::Enable { area }) => area,
        lain => panic!("fokus harus menyalakan IME, bukan {lain:?}"),
    };
    assert_eq!(
        area,
        u.kolom().caret_rect(),
        "kolom di pojok = lokal == global"
    );

    // Caret moves → its area moves with it. An area that does **not** change
    // produces no request at all, so the caret is moved to the front first: the
    // shell is never woken for news it already has.
    u.klik(1.0);
    let area = u.kolom().caret_rect();
    let r = u.tombol(KeyCode::Named(NamedKey::End), Modifiers::NONE);
    let baru = match r.ime {
        Some(ImeRequest::Enable { area }) | Some(ImeRequest::Update { area }) => area,
        lain => panic!("caret pindah harus memperbarui area IME, bukan {lain:?}"),
    };
    assert!(baru.min_x() > area.min_x());

    // Losing focus switches the IME off.
    let r = u.router.focus_node(&mut u.tree, None);
    assert_eq!(r.ime, Some(ImeRequest::Disable));
}

// ---------------------------------------------------------------------------
// Tokens, presets, dark mode
// ---------------------------------------------------------------------------

#[test]
fn warna_dan_bentuk_sudut_selalu_datang_dari_token_di_kedua_preset() {
    let f = fonts();
    for preset in [Preset::Cupertino, Preset::Tailwind] {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let t = Theme::new(preset, appearance);
            let mut u = Uji::baru(text_field(&f, &t, "Nilai").placeholder("kosong"));
            let scene = u.scene();

            let latar = scene
                .commands()
                .iter()
                .find_map(|c| match c {
                    Command::Quad(q) => Some(q.clone()),
                    _ => None,
                })
                .expect("latar kolom");
            assert_eq!(latar.background, t.color.surface);
            assert_eq!(latar.border_color, t.color.border);
            assert!(latar.border_width > 0.0);
            assert_eq!(
                latar.corners.style, t.radius.style,
                "squircle di Cupertino, arc di Tailwind — parameter, bukan konstanta"
            );

            let warna: Vec<_> = scene
                .commands()
                .iter()
                .filter_map(|c| match c {
                    Command::GlyphRun(r) => Some(r.color),
                    _ => None,
                })
                .collect();
            assert_eq!(warna, vec![t.color.label]);
        }
    }
}

#[test]
fn placeholder_memakai_warna_label_tersier_bukan_warna_teks() {
    let f = fonts();
    let t = Theme::tailwind(Appearance::Light);
    let mut u = Uji::baru(text_field(&f, &t, "").placeholder("Cari…"));
    let scene = u.scene();
    let warna: Vec<_> = scene
        .commands()
        .iter()
        .filter_map(|c| match c {
            Command::GlyphRun(r) => Some(r.color),
            _ => None,
        })
        .collect();
    assert_eq!(warna, vec![t.color.tertiary_label]);
    assert_ne!(t.color.tertiary_label, t.color.label);
}

#[test]
fn dark_mode_mengganti_seluruh_warna_tanpa_menyentuh_geometri() {
    let f = fonts();
    let terang = Theme::cupertino(Appearance::Light);
    let gelap = Theme::cupertino(Appearance::Dark);

    let ambil = |t: &Theme| {
        let mut u = Uji::baru(text_field(&f, t, "Nilai"));
        let ukuran = u.ukuran();
        let q = u
            .scene()
            .commands()
            .iter()
            .find_map(|c| match c {
                Command::Quad(q) => Some(q.clone()),
                _ => None,
            })
            .expect("latar");
        (ukuran, q.background, q.corners)
    };
    let (uk_t, bg_t, sudut_t) = ambil(&terang);
    let (uk_g, bg_g, sudut_g) = ambil(&gelap);
    assert_ne!(bg_t, bg_g);
    assert_eq!(uk_t, uk_g);
    assert_eq!(sudut_t, sudut_g);
}

// ---------------------------------------------------------------------------
// Springs & reduced-motion
// ---------------------------------------------------------------------------

#[test]
fn cincin_fokus_tumbuh_lewat_spring_bukan_melompat() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, "isi"));
    let tick = Tick::manual(Duration::from_millis(16), Motion::Full);

    // A field at rest asks for not a single frame.
    assert_eq!(crate::advance(&mut u.tree, &tick), Dirty::NONE);

    u.fokus();
    assert!(crate::is_animating(&u.tree), "fokus harus memulai transisi");

    let mut frame = 0;
    let mut sebelumnya = 0.0f32;
    while crate::advance(&mut u.tree, &tick).contains(Dirty::ANIMATION) {
        let cincin = tebal_cincin(&mut u, t.color.focus_ring);
        assert!(cincin >= sebelumnya - 1e-3, "cincin mundur: {cincin}");
        sebelumnya = cincin;
        frame += 1;
        assert!(frame < 600, "spring tidak pernah berhenti");
    }
    assert!(
        frame > 1,
        "transisi selesai dalam satu frame = itu lompatan"
    );
    // Once settled, the GPU is allowed back to sleep.
    assert_eq!(crate::advance(&mut u.tree, &tick), Dirty::NONE);
    assert!(tebal_cincin(&mut u, t.color.focus_ring) > 0.0);
}

/// The focus-ring thickness actually drawn this frame.
fn tebal_cincin(u: &mut Uji, warna: silka_paint::Color) -> f32 {
    u.scene()
        .commands()
        .iter()
        .filter_map(|c| match c {
            Command::Quad(q) if q.border_color.r == warna.r && q.border_color.a > 0.0 => {
                Some(q.border_width)
            }
            _ => None,
        })
        .fold(0.0f32, f32::max)
}

#[test]
fn reduced_motion_tetap_menjelaskan_tapi_tanpa_pantulan() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, "isi"));
    u.fokus();

    let tick = Tick::manual(Duration::from_millis(16), Motion::Reduced);
    let mut frame = 0;
    while crate::advance(&mut u.tree, &tick).contains(Dirty::ANIMATION) {
        frame += 1;
        assert!(frame < 600);
    }
    // The motion still happens (focus is information, not decoration), and it
    // ends up in the same place full motion would.
    assert!(frame > 0);
    assert!(tebal_cincin(&mut u, t.color.focus_ring) > 0.0);
}

#[test]
fn settle_menyelesaikan_semuanya_seketika() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, "isi"));
    u.fokus();
    assert!(crate::is_animating(&u.tree));
    crate::settle(&mut u.tree);
    assert!(!crate::is_animating(&u.tree));
}

// ---------------------------------------------------------------------------
// Diffing: a controlled value without throwing the caret around
// ---------------------------------------------------------------------------

#[test]
fn rebuild_dengan_nilai_props_yang_sama_tidak_melempar_caret() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, "halo"));
    u.fokus();
    u.klik(1.0);
    assert_eq!(u.kolom().selection().range(), 0..0);

    // A rebuild caused by some other signal: the props are identical, so the
    // field's contents and caret must not be touched at all (the "controlled
    // component" bug).
    let stat = reconcile(&mut u.tree, text_field(&f, &t, "halo").placeholder("beda"));
    assert_eq!(stat.created, 0, "node yang sama, hanya props-nya berganti");
    assert_eq!(u.kolom().selection().range(), 0..0);
    assert_eq!(u.teks(), "halo");
}

#[test]
fn nilai_baru_dari_aplikasi_benar_benar_mengganti_isi() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, "lama"));
    reconcile(&mut u.tree, text_field(&f, &t, "baru"));
    u.tata();
    assert_eq!(u.teks(), "baru");
    assert_eq!(u.kolom().selection().range(), 4..4);
}

#[test]
fn mengetik_lalu_rebuild_dengan_nilai_lama_dari_props_tidak_membatalkan_ketikan() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(text_field(&f, &t, ""));
    u.fokus();
    u.ketik("kete");
    // An app that does **not** wire up `on_change` still gets a typable field:
    // props that never change never overwrite the contents.
    reconcile(&mut u.tree, text_field(&f, &t, ""));
    assert_eq!(u.teks(), "kete");
}

#[test]
fn nilai_yang_dikirim_balik_lewat_on_change_tidak_menggerakkan_caret() {
    let f = fonts();
    let t = tema();
    let nilai = Rc::new(RefCell::new(String::new()));
    let tulis = {
        let n = nilai.clone();
        move |s: &str| *n.borrow_mut() = s.to_string()
    };
    let mut u = Uji::baru(text_field(&f, &t, "").on_change(tulis));
    u.fokus();
    u.ketik("ab");
    // The full round trip: the signal changes → rebuild with the new value.
    let isi = nilai.borrow().clone();
    reconcile(&mut u.tree, text_field(&f, &t, isi.clone()));
    u.tata();
    assert_eq!(isi, "ab");
    assert_eq!(u.teks(), "ab");
    assert_eq!(
        u.kolom().selection().range(),
        2..2,
        "caret harus tetap di ujung ketikan"
    );

    // And typing again continues from there instead of starting over.
    u.ketik("c");
    assert_eq!(u.teks(), "abc");
}

#[test]
fn kotak_seleksi_dan_caret_ikut_bergeser_saat_isi_tergulir() {
    let f = fonts();
    let t = tema();
    let panjang = "kalimat yang jauh lebih panjang daripada lebar kolomnya sendiri, sungguh";
    let mut u = Uji::baru(text_field(&f, &t, panjang));
    u.fokus();
    u.tombol(KeyCode::Named(NamedKey::End), Modifiers::NONE);
    u.tombol(KeyCode::Character('a'), Modifiers::COMMAND);

    let isi = Rect::new(0.0, 0.0, u.ukuran().width, u.ukuran().height);
    for r in u.kolom().selection_rects() {
        assert!(
            r.min_x() >= isi.min_x() - 0.5 && r.max_x() <= isi.max_x() + 0.5,
            "sorotan bocor keluar kolom: {r:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Assistive technology
// ---------------------------------------------------------------------------

#[test]
fn dikte_suara_mengisi_kolom_lewat_aksi_set_value() {
    use silka_core::access::{AccessAction, AccessActionRequest};

    let f = fonts();
    let t = tema();
    let catatan = Catatan::default();
    let mut u = Uji::baru(
        text_field(&f, &t, "")
            .label("Kota")
            .on_change(catatan.rekam()),
    );
    let permintaan = AccessActionRequest {
        target: u.id,
        action: AccessAction::SetValue,
        value: Some("Ubud".into()),
    };
    assert!(apply_access_action(&mut u.tree, &permintaan));
    u.tata();
    assert_eq!(u.teks(), "Ubud");
    assert_eq!(
        catatan.terakhir().as_deref(),
        Some("Ubud"),
        "dikte suara adalah pengguna yang mengetik: aplikasi wajib diberi tahu"
    );
    // The same value produces no second notification.
    assert!(!apply_access_action(&mut u.tree, &permintaan));
    assert_eq!(catatan.jumlah(), 1);
}

#[test]
fn kolom_mati_dan_read_only_menolak_set_value() {
    use silka_core::access::{AccessAction, AccessActionRequest};

    let f = fonts();
    let t = tema();
    for view in [
        text_field(&f, &t, "tetap").disabled(true),
        text_field(&f, &t, "tetap").read_only(true),
    ] {
        let mut u = Uji::baru(view);
        let permintaan = AccessActionRequest {
            target: u.id,
            action: AccessAction::SetValue,
            value: Some("ganti".into()),
        };
        assert!(!apply_access_action(&mut u.tree, &permintaan));
        assert_eq!(u.teks(), "tetap");
    }
}
