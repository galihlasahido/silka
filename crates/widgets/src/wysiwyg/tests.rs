//! `wysiwyg` tests — driven **through the input layer**, never by poking at
//! internal methods.
//!
//! The reasoning is `text_field`'s and `text_area`'s: what has to be proven is
//! not "this function returns that value" but "a user who types, clicks, or
//! presses ⌘B gets the right result". The purely model-level rules are already
//! covered where they live ([`super::document`], [`super::history`],
//! [`super::editor`]), and the line breaker in [`super::layout`]; what is
//! tested here is the widget: the keymap, the toolbar seam, the caret geometry
//! over styled runs, and the accessibility node.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use silka_core::access::{AccessActionRequest, AccessActions, AccessRole};
use silka_core::input::{
    Event, ImeEvent, InputRouter, KeyCode, KeyEvent, Modifiers, NamedKey, PointerButton,
    PointerEvent, PointerPhase, Response,
};
use silka_core::tree::{BoxConstraints, NodeId, RenderTree};
use silka_core::view::{reconcile, View};
use silka_paint::{Point, Size};
use silka_theme::{Appearance, Preset, Theme};

use super::*;
use crate::fonts::Fonts;

const RUANG: Size = Size::new(420.0, 320.0);

fn fonts() -> Fonts {
    Fonts::bundled_only()
}

fn tema() -> Theme {
    Theme::cupertino(Appearance::Dark)
}

fn naskah() -> Document {
    Document::from_blocks(vec![
        Block::plain(BlockKind::Heading1, "Judul"),
        Block::plain(BlockKind::Paragraph, "halo dunia"),
        Block::plain(BlockKind::Bullet, "poin pertama"),
    ])
}

/// One editor inside a tree, wired up to an input router.
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
        let id = first(&tree).expect("badan wysiwyg harus ada di pohon");
        let mut uji = Self {
            tree,
            router: InputRouter::new(),
            id,
            jam: Duration::ZERO,
        };
        uji.fokus();
        uji
    }

    fn dengan(dok: Document) -> Self {
        let f = fonts();
        let t = tema();
        Self::baru(wysiwyg_in(&f, &t, dok).label("Naskah").rows(8))
    }

    fn badan(&self) -> &WysiwygBody {
        self.tree.node_ref::<WysiwygBody>(self.id).expect("badan")
    }

    fn dokumen(&self) -> Document {
        self.badan().document().clone()
    }

    fn tata(&mut self) {
        self.tree.layout(BoxConstraints::loose(RUANG));
    }

    fn sinkron(&mut self) {
        sync(&mut self.tree);
        self.tata();
    }

    fn maju(&mut self, ms: u64) -> Duration {
        self.jam += Duration::from_millis(ms);
        self.jam
    }

    fn fokus(&mut self) -> Response {
        let id = self.id;
        let r = self.router.focus_node(&mut self.tree, Some(id));
        self.tata();
        r
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

    fn perintah(&mut self, c: char, modifiers: Modifiers) -> Response {
        self.tombol(KeyCode::Character(c), modifiers)
    }

    fn ime(&mut self, e: ImeEvent) -> Response {
        let r = self.router.dispatch(&mut self.tree, &Event::Ime(e));
        self.tata();
        r
    }

    /// One full press/release at `titik`, `ms` after the previous event.
    ///
    /// The click **count** is the router's to decide (`ClickConfig`), never the
    /// test's: a double click is two presses close together in time and space,
    /// which is exactly what a user does.
    fn klik(&mut self, titik: Point, ms: u64) {
        for fase in [PointerPhase::Down, PointerPhase::Up] {
            let t = self.maju(if fase == PointerPhase::Down { ms } else { 10 });
            let e = PointerEvent::new(fase, titik, t).button(PointerButton::Primary);
            self.router.dispatch(&mut self.tree, &Event::Pointer(e));
        }
        self.tata();
    }

    fn tekan_di(&mut self, titik: Point) {
        self.klik(titik, 600);
    }

    /// The caret rectangle in the body's own coordinates.
    fn caret(&self) -> silka_paint::Rect {
        self.badan().caret_rect()
    }
}

// ---------------------------------------------------------------------------
// Typing, structure, and undo
// ---------------------------------------------------------------------------

#[test]
fn mengetik_masuk_ke_blok_tempat_caret_berada() {
    let mut u = Uji::dengan(naskah());
    u.ketik("X");
    assert_eq!(u.dokumen().block(0).text(), "XJudul");
    assert_eq!(
        u.dokumen().block(0).kind,
        BlockKind::Heading1,
        "mengetik tidak boleh mengubah jenis blok"
    );
}

#[test]
fn enter_memecah_blok_dan_daftar_tetap_daftar() {
    let mut u = Uji::dengan(naskah());
    // End of the bullet, then Return: the new block is a bullet too.
    u.nama(NamedKey::ArrowDown, Modifiers::COMMAND);
    u.nama(NamedKey::Enter, Modifiers::NONE);
    let d = u.dokumen();
    assert_eq!(d.block_count(), 4);
    assert_eq!(d.block(3).kind, BlockKind::Bullet, "daftar berlanjut");

    // Return again in the now-empty bullet leaves the list.
    u.nama(NamedKey::Enter, Modifiers::NONE);
    assert_eq!(u.dokumen().block(3).kind, BlockKind::Paragraph);
}

#[test]
fn undo_mengembalikan_struktur_blok_bukan_cuma_teks() {
    let mut u = Uji::dengan(naskah());
    // Select from inside the heading to inside the bullet, then type over it.
    u.nama(NamedKey::ArrowRight, Modifiers::NONE);
    for _ in 0..2 {
        u.nama(NamedKey::ArrowDown, Modifiers::SHIFT);
    }
    u.ketik("X");
    assert!(
        u.dokumen().block_count() < 3,
        "seleksi lintas blok terhapus"
    );

    u.perintah('z', Modifiers::COMMAND);
    let d = u.dokumen();
    assert_eq!(d.block_count(), 3, "undo mengembalikan tiga blok");
    assert_eq!(d.block(0).kind, BlockKind::Heading1);
    assert_eq!(d.block(2).kind, BlockKind::Bullet);
    assert_eq!(d.block(2).text(), "poin pertama");
}

#[test]
fn pengetikan_beruntun_dibatalkan_sebagai_satu_langkah() {
    let mut u = Uji::dengan(Document::new());
    u.ketik("halo");
    assert_eq!(u.dokumen().block(0).text(), "halo");
    u.perintah('z', Modifiers::COMMAND);
    assert_eq!(u.dokumen().block(0).text(), "", "satu kata = satu ⌘Z");
    u.perintah('z', Modifiers::COMMAND | Modifiers::SHIFT);
    assert_eq!(u.dokumen().block(0).text(), "halo", "⇧⌘Z mengulanginya");
}

// ---------------------------------------------------------------------------
// Styling
// ---------------------------------------------------------------------------

#[test]
fn menebalkan_sebagian_seleksi_memecah_rentang_gaya() {
    let mut u = Uji::dengan(Document::from_blocks(vec![Block::plain(
        BlockKind::Paragraph,
        "halo dunia",
    )]));
    // Select "halo" with ⇧→ four times, then ⌘B.
    for _ in 0..4 {
        u.nama(NamedKey::ArrowRight, Modifiers::SHIFT);
    }
    u.perintah('b', Modifiers::COMMAND);

    let d = u.dokumen();
    let spans = &d.block(0).spans;
    assert_eq!(spans.len(), 2, "gaya sebagian memecah satu span jadi dua");
    assert_eq!(spans[0].text, "halo");
    assert!(spans[0].style.marks.contains(Marks::BOLD));
    assert_eq!(spans[1].text, " dunia");
    assert!(!spans[1].style.marks.contains(Marks::BOLD));
    assert_eq!(d.block(0).text(), "halo dunia", "teks tidak berubah");
}

#[test]
fn mengetik_di_tengah_tautan_tidak_memperluas_tautan() {
    let dok = Document::from_blocks(vec![Block::new(
        BlockKind::Paragraph,
        vec![Span::new("silka", InlineStyle::link("https://silka.dev"))],
    )]);
    let mut u = Uji::dengan(dok);
    for _ in 0..3 {
        u.nama(NamedKey::ArrowRight, Modifiers::NONE);
    }
    u.ketik("XY");

    let d = u.dokumen();
    assert_eq!(d.block(0).text(), "silXYka");
    let bertaut: String = d
        .block(0)
        .spans
        .iter()
        .filter(|s| s.style.is_link())
        .map(|s| s.text.clone())
        .collect();
    assert_eq!(bertaut, "silka", "teks baru tidak ikut jadi tautan");
}

#[test]
fn perintah_toolbar_dilayani_di_sinkron_frame_berikutnya() {
    let mut u = Uji::dengan(Document::from_blocks(vec![Block::plain(
        BlockKind::Paragraph,
        "satu",
    )]));
    let handle = u.badan().handle().clone();
    u.perintah('a', Modifiers::COMMAND);
    handle.post(EditorCommand::ToggleMark(Marks::ITALIC));
    handle.post(EditorCommand::SetBlockKind(BlockKind::Quote));
    // Nothing has happened yet: commands are queued, never applied on the spot.
    assert!(!u.dokumen().block(0).spans[0]
        .style
        .marks
        .contains(Marks::ITALIC));

    u.sinkron();
    let d = u.dokumen();
    assert!(d.block(0).spans[0].style.marks.contains(Marks::ITALIC));
    assert_eq!(d.block(0).kind, BlockKind::Quote);
}

#[test]
fn keadaan_toolbar_memantulkan_gaya_di_posisi_caret() {
    let dok = Document::from_blocks(vec![Block::new(
        BlockKind::Heading2,
        vec![
            Span::plain("biasa "),
            Span::new("tebal", InlineStyle::with_marks(Marks::BOLD)),
        ],
    )]);
    let terlihat = Rc::new(RefCell::new(EditorSnapshot::default()));
    let salinan = terlihat.clone();
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(
        wysiwyg_in(&f, &t, dok)
            .label("Naskah")
            .rows(6)
            .on_state(move |s| *salinan.borrow_mut() = s.clone()),
    );

    // Caret at the very start: plain text, and the block is a heading.
    u.nama(NamedKey::Home, Modifiers::NONE);
    assert!(!terlihat.borrow().marks.contains(Marks::BOLD));
    assert_eq!(terlihat.borrow().kind, Some(BlockKind::Heading2));

    // Walk into the bold run: the toolbar lights up without anyone asking.
    u.nama(NamedKey::End, Modifiers::NONE);
    assert!(
        terlihat.borrow().marks.contains(Marks::BOLD),
        "tombol tebal harus menyala saat caret di teks tebal"
    );
}

#[test]
fn perintah_k_meminta_dialog_tautan_bukan_membuatnya_sendiri() {
    let diminta = Rc::new(RefCell::new(0u32));
    let hitung = diminta.clone();
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(
        wysiwyg_in(&f, &t, Document::from_plain("halo"))
            .label("Naskah")
            .rows(6)
            .on_link(move || *hitung.borrow_mut() += 1),
    );
    u.perintah('a', Modifiers::COMMAND);
    u.perintah('k', Modifiers::COMMAND);
    assert_eq!(*diminta.borrow(), 1, "⌘K meminta aplikasi membuka dialog");
    assert_eq!(
        u.dokumen(),
        Document::from_plain("halo"),
        "⌘K sendiri belum mengubah apa pun"
    );

    // The dialog answers by posting the address it collected.
    let handle = u.badan().handle().clone();
    handle.post(EditorCommand::SetLink(Some("https://silka.dev".into())));
    u.sinkron();
    assert_eq!(
        u.dokumen().block(0).spans[0].style.link.as_deref(),
        Some("https://silka.dev")
    );

    // And a second ⌘K on the same anchor edits that link rather than nesting a
    // new one inside it.
    handle.post(EditorCommand::SetLink(None));
    u.sinkron();
    assert!(!u.dokumen().block(0).spans[0].style.is_link());
}

#[test]
fn pintasan_control_bekerja_seperti_command() {
    let mut u = Uji::dengan(Document::from_plain("halo"));
    u.perintah('a', Modifiers::CONTROL);
    u.perintah('i', Modifiers::CONTROL);
    assert!(
        u.dokumen().block(0).spans[0]
            .style
            .marks
            .contains(Marks::ITALIC),
        "Ctrl+I harus sama dengan ⌘I di platform tanpa tombol Command"
    );
}

// ---------------------------------------------------------------------------
// Geometry, IME, clipboard
// ---------------------------------------------------------------------------

#[test]
fn klik_menaruh_caret_di_blok_yang_diklik() {
    let mut u = Uji::dengan(naskah());
    let caret_awal = u.caret();
    // Click well below the first block: the caret must leave the heading.
    u.tekan_di(Point::new(20.0, caret_awal.max_y() + 40.0));
    assert!(
        u.badan().selection().focus.block > 0,
        "klik di bawah judul harus pindah blok"
    );
}

#[test]
fn klik_ganda_memilih_kata_klik_tiga_kali_memilih_blok() {
    let mut u = Uji::dengan(naskah());
    // Walk the caret into the middle of "halo dunia", then click exactly where
    // it stands: the geometry under test is the widget's own, so the test must
    // not guess at coordinates.
    u.nama(NamedKey::ArrowDown, Modifiers::NONE);
    for _ in 0..2 {
        u.nama(NamedKey::ArrowRight, Modifiers::NONE);
    }
    let titik = u.caret().center();
    u.tekan_di(titik);
    u.klik(titik, 80);
    let sel = u.badan().selection();
    assert!(!sel.is_collapsed(), "klik ganda memilih satu kata");
    let r = sel.range();
    assert_eq!(r.start.block, r.end.block);

    u.klik(titik, 80);
    let sel = u.badan().selection();
    let blok = sel.focus.block;
    assert_eq!(sel.range().start.offset, 0);
    assert_eq!(sel.range().end.offset, u.dokumen().block(blok).len());
}

#[test]
fn caret_bergerak_ke_kanan_saat_mengetik_di_dalam_teks_bergaya() {
    let dok = Document::from_blocks(vec![Block::new(
        BlockKind::Paragraph,
        vec![
            Span::new("tebal", InlineStyle::with_marks(Marks::BOLD)),
            Span::plain(" biasa"),
        ],
    )]);
    let mut u = Uji::dengan(dok);
    let awal = u.caret().origin.x;
    for _ in 0..5 {
        u.nama(NamedKey::ArrowRight, Modifiers::NONE);
    }
    let setelah_tebal = u.caret().origin.x;
    assert!(
        setelah_tebal > awal,
        "caret harus bergerak melewati rentang tebal: {awal} -> {setelah_tebal}"
    );
}

#[test]
fn preedit_ime_di_tengah_teks_bergaya_belum_sampai_ke_dokumen() {
    let dok = Document::from_blocks(vec![Block::new(
        BlockKind::Paragraph,
        vec![Span::new("tebal", InlineStyle::with_marks(Marks::BOLD))],
    )]);
    let mut u = Uji::dengan(dok);
    for _ in 0..3 {
        u.nama(NamedKey::ArrowRight, Modifiers::NONE);
    }
    u.ime(ImeEvent::Enabled);
    u.ime(ImeEvent::Preedit {
        text: "にほn".into(),
        cursor: None,
    });
    assert_eq!(
        u.dokumen().block(0).text(),
        "tebal",
        "komposisi belum jadi isi"
    );
    assert!(u.badan().is_composing());

    u.ime(ImeEvent::Commit("日本".into()));
    let d = u.dokumen();
    assert_eq!(d.block(0).text(), "teb日本al");
    assert!(
        d.block(0)
            .spans
            .iter()
            .all(|s| s.style.marks.contains(Marks::BOLD)),
        "teks yang dikomit mewarisi gaya di sekelilingnya: {:?}",
        d.block(0).spans
    );
}

#[test]
fn salin_menghasilkan_dua_rasa_dan_yang_kaya_bisa_ditempel_lagi() {
    let disalin = Rc::new(RefCell::new(Clipping::default()));
    let salinan = disalin.clone();
    let f = fonts();
    let t = tema();
    let dok = Document::from_blocks(vec![
        Block::plain(BlockKind::Heading1, "Judul"),
        Block::new(
            BlockKind::Bullet,
            vec![Span::new("poin", InlineStyle::with_marks(Marks::BOLD))],
        ),
    ]);
    let mut u = Uji::baru(
        wysiwyg_in(&f, &t, dok)
            .label("Naskah")
            .rows(8)
            .on_copy(move |c| *salinan.borrow_mut() = c.clone()),
    );

    u.perintah('a', Modifiers::COMMAND);
    u.perintah('c', Modifiers::COMMAND);
    let c = disalin.borrow().clone();
    assert_eq!(c.plain, "Judul\n• poin", "ke luar aplikasi: teks polos");

    let potongan = decode(&c.rich).expect("rasa kaya harus format internal");
    assert_eq!(potongan.pieces.len(), 2);
    assert_eq!(potongan.pieces[1].kind, BlockKind::Bullet);
    assert!(potongan.pieces[1].spans[0]
        .style
        .marks
        .contains(Marks::BOLD));

    // Paste it back at the end: the styles and the block kinds survive.
    u.nama(NamedKey::ArrowDown, Modifiers::COMMAND);
    let handle = u.badan().handle().clone();
    handle.post(EditorCommand::InsertFragment(potongan));
    u.sinkron();
    let d = u.dokumen();
    assert_eq!(d.block_count(), 3);
    assert_eq!(d.block(2).kind, BlockKind::Bullet);
    assert!(d.block(2).spans[0].style.marks.contains(Marks::BOLD));
}

// ---------------------------------------------------------------------------
// Definition of Done
// ---------------------------------------------------------------------------

#[test]
fn node_aksesibilitas_melaporkan_peran_multiline_caret_dan_seleksi() {
    let mut u = Uji::dengan(naskah());
    u.perintah('a', Modifiers::COMMAND);

    let mut node = silka_core::access::AccessNode::default();
    u.badan().access(&mut node);
    assert_eq!(node.role, AccessRole::MultilineTextInput);
    assert_eq!(node.label.as_deref(), Some("Naskah"));
    assert_eq!(
        node.value.as_deref(),
        Some("Judul\nhalo dunia\npoin pertama")
    );
    let sel = node.text_selection.expect("caret harus dilaporkan");
    assert_eq!(sel.anchor, 0);
    assert_eq!(
        sel.focus,
        "Judul\nhalo dunia\npoin pertama".chars().count(),
        "seleksi ⌘A menutupi seluruh dokumen"
    );
    assert!(node.actions.contains(AccessActions::SET_VALUE));
}

#[test]
fn dikte_mengganti_isi_lewat_aksi_set_value() {
    let mut u = Uji::dengan(naskah());
    let id = u.id;
    let permintaan = AccessActionRequest {
        target: id,
        action: silka_core::access::AccessAction::SetValue,
        value: Some("baris satu\nbaris dua".into()),
    };
    assert!(apply_access_action(&mut u.tree, &permintaan));
    let d = u.dokumen();
    assert_eq!(d.block_count(), 2);
    assert_eq!(d.block(1).text(), "baris dua");
}

#[test]
fn editor_read_only_bisa_dibaca_tapi_tidak_bisa_diubah() {
    let f = fonts();
    let t = tema();
    let mut u = Uji::baru(
        wysiwyg_in(&f, &t, naskah())
            .label("Naskah")
            .rows(6)
            .read_only(true),
    );
    u.ketik("x");
    u.perintah('b', Modifiers::COMMAND);
    u.sinkron();
    assert_eq!(u.dokumen(), naskah(), "read-only tidak boleh berubah");

    let mut node = silka_core::access::AccessNode::default();
    u.badan().access(&mut node);
    assert!(!node.actions.contains(AccessActions::SET_VALUE));
}

#[test]
fn tab_meninggalkan_editor_bukan_ditelan_olehnya() {
    let mut u = Uji::dengan(naskah());
    u.nama(NamedKey::Tab, Modifiers::NONE);
    // The editor does not swallow Tab: no tab character lands in the document,
    // and focus is handed to the navigation layer. A text box that eats Tab is
    // a keyboard trap (`KOMPONEN.md` DoD, §3.8).
    assert_eq!(u.dokumen(), naskah(), "Tab tidak boleh menjadi isi");
}

#[test]
fn benar_di_kedua_preset_dan_kedua_mode() {
    let f = fonts();
    for preset in [Preset::Cupertino, Preset::Tailwind] {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let t = Theme::new(preset, appearance);
            let gaya = EditorStyle::from_theme(&t);
            assert_eq!(gaya.text, t.color.label);
            assert_eq!(gaya.link, t.color.accent);
            assert_eq!(gaya.code_corners, t.corners(t.radius.sm));
            let mut u = Uji::baru(wysiwyg_in(&f, &t, naskah()).label("Naskah").rows(6));
            u.ketik("x");
            assert_eq!(u.dokumen().block(0).text(), "xJudul");
        }
    }
}

#[test]
fn editor_tumbuh_bersama_isinya_sampai_batas() {
    let f = fonts();
    let t = tema();
    let e = wysiwyg_in(&f, &t, Document::new()).auto_grow(2, 6);
    let (min, max) = e.height_range();
    assert!(min < max, "auto_grow punya jangkauan: {min}..{max}");
    assert!(min >= crate::button::MIN_HIT_TARGET, "tetap bisa disentuh");
}

// ---------------------------------------------------------------------------
// The incremental block layout
// ---------------------------------------------------------------------------

/// The layout cache ([`layout::rebuild`]) is what makes a long note typable at
/// all — before it, one keystroke re-shaped every block in the document — so it
/// gets its own tests rather than riding on the widget's.
mod tata_letak_bertahap {
    use super::*;
    use crate::wysiwyg::layout::{block_key, rebuild, BlockInput, DocLayout, EditorStyle};
    use silka_paint::{Point, Rect};

    /// The blocks a test document is made of.
    fn blok(teks: &[(BlockKind, &str)]) -> Vec<Block> {
        teks.iter()
            .map(|(kind, t)| Block::plain(*kind, *t))
            .collect()
    }

    /// Lay out `blocks`, reusing `previous` when there is one.
    fn susun(blocks: &[Block], previous: Option<DocLayout>) -> DocLayout {
        let f = fonts();
        let gaya = EditorStyle::from_theme(&tema());
        let masukan: Vec<BlockInput<'_>> = blocks
            .iter()
            .enumerate()
            .map(|(i, b)| BlockInput {
                kind: b.kind,
                spans: &b.spans,
                number: if b.kind == BlockKind::Numbered {
                    i + 1
                } else {
                    1
                },
            })
            .collect();
        f.with(|m| {
            rebuild(
                m,
                &masukan,
                &gaya,
                320.0,
                false,
                Point::new(8.0, 6.0),
                previous,
            )
        })
    }

    /// Every glyph rectangle of a laid-out document, in order.
    fn kotak_glyph(l: &DocLayout) -> Vec<Rect> {
        let mut out = Vec::new();
        for b in &l.blocks {
            if let Some(m) = &b.marker {
                out.extend(m.run.glyphs.iter().map(|g| g.bounds));
            }
            for baris in &b.lines {
                for seg in &baris.segments {
                    out.extend(seg.run.glyphs.iter().map(|g| g.bounds));
                }
            }
        }
        out
    }

    #[test]
    fn kunci_blok_menutupi_semua_yang_dibaca_pembentuk() {
        let polos = Block::plain(BlockKind::Paragraph, "halo");
        let tebal = Block::new(
            BlockKind::Paragraph,
            vec![Span::new("halo", InlineStyle::with_marks(Marks::BOLD))],
        );
        let tautan = Block::new(
            BlockKind::Paragraph,
            vec![Span::new("halo", InlineStyle::link("https://a"))],
        );
        let judul = Block::plain(BlockKind::Heading1, "halo");

        let kunci = |b: &Block, nomor: usize, nonaktif: bool| {
            block_key(
                &BlockInput {
                    kind: b.kind,
                    spans: &b.spans,
                    number: nomor,
                },
                nonaktif,
            )
        };

        // Sama isi, sama kunci — itulah yang membuat pemakaian ulang aman.
        assert_eq!(kunci(&polos, 1, false), kunci(&polos.clone(), 1, false));
        // …dan setiap hal yang mengubah glyph mengubah kunci.
        assert_ne!(kunci(&polos, 1, false), kunci(&tebal, 1, false));
        assert_ne!(kunci(&polos, 1, false), kunci(&tautan, 1, false));
        assert_ne!(kunci(&polos, 1, false), kunci(&judul, 1, false));
        assert_ne!(kunci(&polos, 1, false), kunci(&polos, 2, false));
        assert_ne!(kunci(&polos, 1, false), kunci(&polos, 1, true));

        // Batas antar-span ikut terhitung: ["ab","c"] bukan ["a","bc"].
        let a = Block::new(
            BlockKind::Paragraph,
            vec![
                Span::new("ab", InlineStyle::with_marks(Marks::BOLD)),
                Span::plain("c"),
            ],
        );
        let b = Block::new(
            BlockKind::Paragraph,
            vec![
                Span::new("a", InlineStyle::with_marks(Marks::BOLD)),
                Span::plain("bc"),
            ],
        );
        assert_ne!(kunci(&a, 1, false), kunci(&b, 1, false));
    }

    #[test]
    fn menyisipkan_blok_di_atas_tidak_menggeser_glyph_secara_salah() {
        // The whole point of matching blocks **by content** instead of by
        // index: inserting a paragraph at the top moves everything below it,
        // and the moved blocks must land exactly where a fresh layout would
        // have put them.
        let awal = blok(&[
            (BlockKind::Paragraph, "satu"),
            (BlockKind::Bullet, "dua"),
            (BlockKind::Quote, "tiga"),
        ]);
        let sebelumnya = susun(&awal, None);

        let mut sesudah = vec![Block::plain(BlockKind::Heading1, "Judul baru")];
        sesudah.extend(awal.iter().cloned());

        let bertahap = susun(&sesudah, Some(sebelumnya));
        let dari_nol = susun(&sesudah, None);

        assert_eq!(bertahap.size, dari_nol.size);
        assert_eq!(kotak_glyph(&bertahap), kotak_glyph(&dari_nol));
    }

    #[test]
    fn mengubah_satu_blok_tidak_merusak_tetangganya() {
        let awal = blok(&[
            (BlockKind::Paragraph, "satu"),
            (BlockKind::Paragraph, "dua"),
            (BlockKind::Paragraph, "tiga"),
        ]);
        let sebelumnya = susun(&awal, None);

        let mut sesudah = awal.clone();
        sesudah[1] = Block::plain(BlockKind::Paragraph, "dua dengan tambahan");

        let bertahap = susun(&sesudah, Some(sebelumnya));
        let dari_nol = susun(&sesudah, None);
        assert_eq!(bertahap.size, dari_nol.size);
        assert_eq!(kotak_glyph(&bertahap), kotak_glyph(&dari_nol));
    }

    #[test]
    fn menghapus_blok_menarik_sisanya_ke_atas() {
        let awal = blok(&[
            (BlockKind::Heading1, "Judul"),
            (BlockKind::Paragraph, "satu"),
            (BlockKind::Paragraph, "dua"),
        ]);
        let sebelumnya = susun(&awal, None);

        let sesudah = vec![awal[0].clone(), awal[2].clone()];
        let bertahap = susun(&sesudah, Some(sebelumnya));
        let dari_nol = susun(&sesudah, None);
        assert_eq!(bertahap.size, dari_nol.size);
        assert_eq!(kotak_glyph(&bertahap), kotak_glyph(&dari_nol));
    }

    #[test]
    fn blok_kembar_tidak_saling_tertukar() {
        // Two identical paragraphs share a key, so the cache has to hand out
        // each cached block **once** — handing the same one out twice would
        // draw one paragraph in two places and leave a hole.
        let awal = blok(&[
            (BlockKind::Paragraph, "sama"),
            (BlockKind::Paragraph, "sama"),
            (BlockKind::Paragraph, "beda"),
        ]);
        let sebelumnya = susun(&awal, None);
        let bertahap = susun(&awal, Some(sebelumnya));
        let dari_nol = susun(&awal, None);
        assert_eq!(kotak_glyph(&bertahap), kotak_glyph(&dari_nol));
    }
}
