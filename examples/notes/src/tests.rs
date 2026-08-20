//! The application's behaviour tests.
//!
//! Every one of them drives **the application that ships**: [`Shell`] is what
//! `main` opens a window around, with the same `Env`, the same shortcut table
//! and the same once-a-frame pump. What is different is only the clock (fake,
//! so a test never depends on how fast the machine is), the pasteboard
//! ([`Mode::InProcess`], because CI has none) and the notes directory (a temp
//! directory that removes itself).
//!
//! Four of the tests below are the ones this application was written to answer,
//! and each of them is a question a unit test of `wysiwyg` cannot ask:
//!
//! | Question | Test |
//! |---|---|
//! | Does a thousand-paragraph note stay responsive? | [`a_thousand_paragraph_note_stays_responsive`] |
//! | Do fifty undos restore the block structure? | [`fifty_undos_walk_the_block_structure_back`] |
//! | Does text from another application arrive plain? | [`pasting_from_outside_the_application_lands_as_plain_text`] |
//! | Does switching notes with unsaved writing lose it? | [`switching_notes_with_unsaved_writing_keeps_every_word`] |

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use silka_core::access::{AccessRole, AccessTree};
use silka_core::input::{
    Event, KeyCode, KeyEvent, Modifiers, NamedKey, PointerButton, PointerEvent, PointerPhase,
};
use silka_paint::{Point, Rect, Size};
use silka_theme::{Appearance, Preset, Theme};
use silka_widgets::wysiwyg::{Block, BlockKind, Document};
use silka_widgets::{install_fonts, Fonts, TreeKey};

use crate::app::{self, Shell, Shortcut, EDITOR_LABEL};
use crate::markdown;
use crate::pasteboard::Mode;
use crate::sidebar;
use crate::state::{self, SaveStatus};
use crate::stats;
use crate::store;

/// The window the tests pretend to be.
const VIEWPORT: Size = Size::new(1200.0, 820.0);
/// The gap between test frames — 120 Hz, what a ProMotion display link
/// reports. A **fake clock**, never `Instant::now()` (REKOMENDASI §9.5).
const FRAME: Duration = Duration::from_millis(8);

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// A notes directory that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "silka-notes-app-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("temp dir");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The application under test, plus its clock and its directory.
struct Screen {
    #[allow(dead_code)]
    scratch: Scratch,
    shell: Shell,
    clock: Instant,
}

impl Screen {
    /// The seeded sample library — four notes in two folders.
    fn new(name: &str) -> Self {
        Self::with(name, |root| {
            store::seed(root).expect("seed");
        })
    }

    /// A library the caller writes itself.
    fn with(name: &str, prepare: impl FnOnce(&Path)) -> Self {
        // One text engine for the whole process, exactly as `main` installs it:
        // without it every label renders blank and every measurement is wrong.
        let fonts = Fonts::new();
        install_fonts(&fonts);

        let scratch = Scratch::new(name);
        prepare(scratch.path());
        let library = store::scan(scratch.path()).expect("scan");
        let shell = Shell::new(Theme::cupertino(Appearance::Dark), library, Mode::InProcess)
            .sized(VIEWPORT.width, VIEWPORT.height);
        let mut screen = Screen {
            scratch,
            shell,
            clock: Instant::now(),
        };
        screen.quiesce();
        screen
    }

    /// One complete frame: background work first, then the animation tick, then
    /// rebuild → layout → paint — the order the window uses.
    fn frame(&mut self) {
        self.clock += FRAME;
        self.shell.pump(self.clock);
        self.shell.ui.animate_at(self.clock, silka_widgets::advance);
        self.shell.ui.frame();
    }

    /// Pump frames until nothing is left to do, background work included.
    ///
    /// The cap is deliberate: work that never finishes has to be a failure
    /// rather than a hang.
    fn quiesce(&mut self) {
        for _ in 0..900 {
            if !self.shell.ui.tasks().is_idle() {
                // Waiting rather than sleeping: `wait_for_idle` returns the
                // moment every worker has handed its payload over, which makes
                // the suite deterministic instead of merely usually green.
                self.shell.ui.tasks().wait_for_idle();
            }
            self.frame();
            if self.shell.ui.is_idle() && self.shell.ui.tasks().is_idle() {
                return;
            }
        }
        panic!("something in the notes application never stops moving");
    }

    /// Let time pass without any input, so the auto-save debounce elapses.
    fn wait(&mut self, duration: Duration) {
        self.clock += duration;
        self.quiesce();
    }

    fn tree(&self) -> AccessTree {
        self.shell.ui.access_tree()
    }

    fn rect(&self, label: &str) -> Rect {
        let tree = self.tree();
        tree.find_label(label)
            .unwrap_or_else(|| panic!("tidak ada node berlabel {label:?}:\n{}", tree.dump()))
            .bounds
    }

    fn value(&self, label: &str) -> String {
        let tree = self.tree();
        tree.find_label(label)
            .and_then(|e| e.node.value.clone())
            .unwrap_or_else(|| panic!("{label:?} tanpa nilai:\n{}", tree.dump()))
    }

    fn has(&self, label: &str) -> bool {
        self.tree().find_label(label).is_some()
    }

    /// Every label in the accessibility tree — what a screen reader would read.
    fn labels(&self) -> Vec<String> {
        self.tree()
            .entries()
            .iter()
            .filter_map(|e| e.node.label.clone())
            .collect()
    }

    fn click(&mut self, point: Point) {
        for e in [
            PointerEvent::new(PointerPhase::Move, point, Duration::ZERO),
            PointerEvent::new(PointerPhase::Down, point, Duration::from_millis(8))
                .button(PointerButton::Primary),
            PointerEvent::new(PointerPhase::Up, point, Duration::from_millis(60))
                .button(PointerButton::Primary),
        ] {
            self.shell.dispatch(&Event::Pointer(e));
        }
        self.frame();
    }

    fn click_label(&mut self, label: &str) {
        self.click(self.rect(label).center());
    }

    /// Put the caret in the editor.
    fn focus_editor(&mut self) {
        let box_ = self.rect(EDITOR_LABEL);
        // Near the top-left of the body, which is inside the first block
        // whatever the note is.
        self.click(Point::new(box_.origin.x + 40.0, box_.origin.y + 14.0));
        self.frame();
    }

    /// One key, and one complete frame after it.
    fn key(&mut self, code: KeyCode, modifiers: Modifiers) {
        self.press(code, modifiers);
        self.frame();
    }

    /// One key **without** a frame.
    ///
    /// The editor applies a keystroke during dispatch, so a burst of typing
    /// does not need a frame between the characters — and in a debug build it
    /// had better not have one: a keystroke that changes the block count makes
    /// the editor ask for a relayout, and a relayout is not stopped by a
    /// component boundary (`ComponentBox` is transparent on purpose), so it
    /// reaches the root and re-lays out the toolbar's nine buttons with it.
    /// That is cheap in a release build and emphatically not in this one.
    fn press(&mut self, code: KeyCode, modifiers: Modifiers) {
        self.shell.dispatch(&Event::Key(
            KeyEvent::pressed(code, Duration::from_millis(8)).modifiers(modifiers),
        ));
    }

    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            let code = match c {
                ' ' => KeyCode::Named(NamedKey::Space),
                '\n' => KeyCode::Named(NamedKey::Enter),
                c => KeyCode::Character(c),
            };
            self.press(code, Modifiers::NONE);
        }
        self.frame();
    }

    /// The note that is open.
    fn open_note(&self) -> Option<TreeKey> {
        self.shell.store().open.peek()
    }

    /// The document of a note as the application currently holds it.
    fn document(&self, note: TreeKey) -> Option<Document> {
        self.shell
            .store()
            .docs
            .peek_with(|d| d.document(note).cloned())
    }

    /// The note whose title is `title`.
    fn note_named(&self, title: &str) -> TreeKey {
        self.shell
            .store()
            .library
            .peek_with(|l| l.notes().iter().find(|n| n.title == title).map(|n| n.id))
            .unwrap_or_else(|| panic!("tidak ada catatan berjudul {title:?}"))
    }

    /// The file behind a note.
    fn file_of(&self, note: TreeKey) -> PathBuf {
        self.shell
            .store()
            .library
            .peek_with(|l| l.note(note).map(|n| n.file.clone()))
            .expect("catatan punya berkas")
    }

    /// Open a note the way the palette does.
    fn open(&mut self, note: TreeKey) {
        sidebar::select_note(self.shell.chrome().outline, &self.shell.store(), note);
        self.shell.store().open_note(note);
        self.quiesce();
    }
}

// ---------------------------------------------------------------------------
// The four proofs
// ---------------------------------------------------------------------------

/// A note long enough that anything with the wrong complexity in it shows up.
fn long_note(paragraphs: usize) -> String {
    let mut out = String::from("# A very long note\n\n");
    for i in 0..paragraphs {
        out.push_str(&format!(
            "Paragraph number {i} of this note, with enough words in it to wrap \
             and to be worth shaping.\n\n"
        ));
    }
    out
}

#[test]
fn a_thousand_paragraph_note_stays_responsive() {
    // The claim: typing into a note of more than a thousand paragraphs costs
    // what the *keystroke* costs, not what the *document* costs.
    //
    // It did not, and finding that out is what this application is for. The
    // editor re-shaped every block of the document on every edit, so a
    // keystroke in a 1200-paragraph note took the better part of a second. The
    // fix is in the widget (`wysiwyg::layout::rebuild`): blocks are matched by
    // content between frames and only the ones that changed are shaped again.
    //
    // The budget below is generous on purpose — it is measured in a debug
    // build, on whatever machine CI happens to be — but it is two orders of
    // magnitude under the behaviour it is guarding against, which is what makes
    // it a regression test rather than a benchmark.
    const PARAGRAPHS: usize = 1200;
    const BUDGET: Duration = Duration::from_millis(120);

    let mut screen = Screen::with("long", |root| {
        fs::write(root.join("Long.md"), long_note(PARAGRAPHS)).expect("write");
    });
    let note = screen.note_named("Long");
    screen.open(note);

    let document = screen.document(note).expect("dokumen termuat");
    assert!(
        document.block_count() > 1000,
        "catatan uji harus lebih dari 1000 blok, bukan {}",
        document.block_count()
    );

    screen.focus_editor();

    // Ten characters, each one a complete frame: dispatch, rebuild, layout,
    // paint. The first is allowed to be slower than the rest (the caret has
    // just landed and the focus ring is starting to move), so the measurement
    // is the median of the ten rather than their total.
    let mut samples: Vec<Duration> = Vec::new();
    for c in "responsive".chars() {
        let started = Instant::now();
        screen.key(KeyCode::Character(c), Modifiers::NONE);
        samples.push(started.elapsed());
    }
    samples.sort();
    let median = samples[samples.len() / 2];
    assert!(
        median < BUDGET,
        "satu ketukan di catatan {PARAGRAPHS} paragraf makan {median:?} \
         (anggaran {BUDGET:?}); sampel: {samples:?}"
    );

    // And it really typed into the document rather than merely being fast.
    let after = screen.document(note).expect("dokumen masih ada");
    assert_eq!(after.block_count(), document.block_count());
    assert!(
        after.plain_text().contains("responsive"),
        "sepuluh ketukan harus sampai ke dokumen"
    );
}

#[test]
fn fifty_undos_walk_the_block_structure_back() {
    // Fifty separate operations, each one a Return that splits a block, then
    // fifty ⌘Z. What has to come back is the **structure** — the block count
    // and every block's kind — not merely the characters.
    let mut screen = Screen::with("undo", |root| {
        fs::write(
            root.join("Undo.md"),
            "# Heading\n\n- one\n- two\n\n> a quote\n\n```\ncode line\n```\n\nlast paragraph\n",
        )
        .expect("write");
    });
    let note = screen.note_named("Undo");
    screen.open(note);
    screen.focus_editor();

    let before = screen.document(note).expect("dokumen termuat");
    let kinds_before: Vec<BlockKind> = before.blocks().iter().map(|b| b.kind).collect();
    let text_before = before.plain_text();

    // ⌘End puts the caret at the very end, so every Return splits the last
    // block and the operations stay independent of each other.
    screen.key(KeyCode::Named(NamedKey::End), Modifiers::COMMAND);
    for i in 0..50 {
        screen.press(KeyCode::Named(NamedKey::Enter), Modifiers::NONE);
        screen.press(
            KeyCode::Character(char::from(b'a' + (i % 26) as u8)),
            Modifiers::NONE,
        );
    }
    screen.frame();
    let grown = screen.document(note).expect("dokumen ada");
    assert_eq!(
        grown.block_count(),
        before.block_count() + 50,
        "lima puluh Return harus menambah lima puluh blok"
    );

    // A hundred undos: fifty typed characters and fifty splits. Typing one
    // character at a time is one step each here, because each Return between
    // them closes the previous run.
    for _ in 0..100 {
        screen.press(KeyCode::Character('z'), Modifiers::COMMAND);
    }
    screen.frame();

    let after = screen.document(note).expect("dokumen ada");
    let kinds_after: Vec<BlockKind> = after.blocks().iter().map(|b| b.kind).collect();
    assert_eq!(
        kinds_after, kinds_before,
        "undo harus mengembalikan jenis setiap blok, bukan sekadar teksnya"
    );
    assert_eq!(after.plain_text(), text_before);
    assert_eq!(after, before, "dokumen harus identik dengan sebelum diedit");
}

#[test]
fn pasting_from_outside_the_application_lands_as_plain_text() {
    // Something that *looks* like this application's own content — Markdown
    // with a heading and bold text in it — arrives from another application.
    // It has to land as the characters it is, not as a heading.
    const FOREIGN: &str = "# Not a heading\nand **not bold** either";

    let mut screen = Screen::with("paste", |root| {
        fs::write(root.join("Paste.md"), "the paste lands here\n").expect("write");
    });
    let note = screen.note_named("Paste");
    screen.open(note);
    screen.focus_editor();
    screen.key(KeyCode::Named(NamedKey::End), Modifiers::COMMAND);

    let before = screen.document(note).expect("dokumen termuat");
    screen
        .shell
        .chrome()
        .pasteboard
        .update(|p| p.set_external(FOREIGN));
    screen.key(KeyCode::Character('v'), Modifiers::COMMAND);
    // The paste is served by `wysiwyg::sync` on the next frame, exactly like a
    // toolbar command.
    screen.frame();

    let after = screen.document(note).expect("dokumen ada");
    let text = after.plain_text();
    assert!(
        text.contains("# Not a heading"),
        "tanda pagar harus tetap jadi huruf, bukan jadi judul: {text:?}"
    );
    assert!(
        text.contains("**not bold**"),
        "bintang harus tetap jadi huruf: {text:?}"
    );
    // Nothing gained a mark, and no block became a heading.
    assert!(
        after
            .blocks()
            .iter()
            .all(|b| b.kind == BlockKind::Paragraph),
        "tempelan luar tidak boleh mengubah jenis blok: {:?}",
        after.blocks().iter().map(|b| b.kind).collect::<Vec<_>>()
    );
    assert!(
        after
            .blocks()
            .iter()
            .flat_map(|b| b.spans.iter())
            .all(|s| s.style.marks.is_empty() && !s.style.is_link()),
        "tempelan luar tidak boleh membawa gaya"
    );
    assert!(after.block_count() > before.block_count());
}

#[test]
fn switching_notes_with_unsaved_writing_keeps_every_word() {
    // Type into one note, switch to another **before the auto-save debounce
    // has elapsed**, then come back. Two things must hold: the writing is
    // still in the editor, and it reached the file.
    let mut screen = Screen::new("switch");
    let first = screen.note_named("Welcome");
    let second = screen.note_named("Roadmap");

    screen.open(first);
    screen.focus_editor();
    screen.key(KeyCode::Named(NamedKey::End), Modifiers::COMMAND);
    screen.type_text(" UNSAVED");

    // Not saved yet: the debounce has not run and the status bar says so.
    assert_eq!(screen.shell.store().status(), SaveStatus::Pending(1));
    let on_disk = fs::read_to_string(screen.file_of(first)).expect("read");
    assert!(
        !on_disk.contains("UNSAVED"),
        "prasyarat uji: tulisan belum sampai ke berkas"
    );

    // Switch — the pump notices the open note changed and flushes.
    screen.open(second);
    assert_eq!(
        screen.open_note(),
        Some(second),
        "catatan kedua harus terbuka"
    );

    // In memory, first.
    let kept = screen
        .document(first)
        .expect("penyangga catatan pertama tetap ada");
    assert!(
        kept.plain_text().contains("UNSAVED"),
        "berpindah catatan tidak boleh membuang tulisan yang belum tersimpan"
    );
    // …and on disk.
    let on_disk = fs::read_to_string(screen.file_of(first)).expect("read");
    assert!(
        on_disk.contains("UNSAVED"),
        "berpindah catatan harus menyiram tulisan ke berkas:\n{on_disk}"
    );
    assert_eq!(screen.shell.store().status(), SaveStatus::Saved);

    // Coming back shows it, and reading the file back parses it the same way.
    screen.open(first);
    assert!(screen.value(EDITOR_LABEL).contains("UNSAVED"));
    assert_eq!(markdown::from_markdown(&on_disk), kept);
}

// ---------------------------------------------------------------------------
// The rest of the application
// ---------------------------------------------------------------------------

#[test]
fn the_window_opens_on_a_note_with_an_outline_beside_it() {
    let screen = Screen::new("open");
    assert!(screen.has(sidebar::OUTLINE_LABEL), "{:?}", screen.labels());
    assert!(screen.has(sidebar::SEARCH_LABEL));
    assert!(screen.has(app::SPLIT_LABEL));

    let editor = screen
        .tree()
        .find_label(EDITOR_LABEL)
        .expect("editor ada")
        .node
        .clone();
    assert_eq!(editor.role, AccessRole::MultilineTextInput);
    assert!(editor.text_selection.is_some(), "caret harus dilaporkan");
    assert!(screen.value(EDITOR_LABEL).starts_with("Welcome"));

    // The outline really is a tree, with the folders as branches.
    let roles: Vec<AccessRole> = screen
        .tree()
        .entries()
        .iter()
        .map(|e| e.node.role)
        .collect();
    assert!(roles.contains(&AccessRole::Tree), "{roles:?}");
    assert!(screen.labels().iter().any(|l| l == "Projects"));
}

#[test]
fn the_editor_is_wide_enough_to_be_the_main_pane() {
    let screen = Screen::new("split");
    let outline = screen.rect(sidebar::OUTLINE_LABEL);
    let editor = screen.rect(EDITOR_LABEL);
    assert!(
        editor.size.width > outline.size.width,
        "editor {editor:?} harus lebih lebar daripada daftar {outline:?}"
    );
    assert!(
        editor.origin.x > outline.origin.x,
        "editor ada di kanan daftar"
    );
}

#[test]
fn the_word_count_follows_the_writing() {
    let mut screen = Screen::new("words");
    let note = screen.note_named("Welcome");
    screen.open(note);

    let counted = stats::count(&screen.document(note).expect("dokumen"));
    assert!(counted.words > 20, "{counted:?}");
    assert!(
        screen.labels().iter().any(|l| l == &counted.summary()),
        "baris status harus menyebut hitungan kata: {:?}",
        screen.labels()
    );

    screen.focus_editor();
    screen.key(KeyCode::Named(NamedKey::End), Modifiers::COMMAND);
    screen.type_text(" plus three more words");
    let after = stats::count(&screen.document(note).expect("dokumen"));
    assert_eq!(after.words, counted.words + 4);
    assert!(screen.labels().iter().any(|l| l == &after.summary()));
}

#[test]
fn the_search_field_turns_the_outline_into_results() {
    let mut screen = Screen::new("search");
    // The index is built by a background task; `Screen::new` already pumped
    // until it landed.
    assert!(screen.shell.store().index.peek_with(|i| i.is_ready()));

    screen.shell.store().query.set("disbursement".to_string());
    screen.quiesce();
    assert!(
        screen.has(sidebar::NO_MATCHES),
        "kata yang tidak ada harus memberi hasil kosong: {:?}",
        screen.labels()
    );

    screen.shell.store().query.set("roadmap".to_string());
    screen.quiesce();
    assert!(screen.labels().iter().any(|l| l == "Roadmap"));
    assert!(
        !screen.labels().iter().any(|l| l == "Projects"),
        "saat mencari, folder tidak ditampilkan: {:?}",
        screen.labels()
    );

    // A word that only appears in a note's *body*, never in its title.
    screen.shell.store().query.set("sidebar".to_string());
    screen.quiesce();
    assert!(
        screen.labels().iter().any(|l| l == "Silka"),
        "pencarian harus menembus isi catatan: {:?}",
        screen.labels()
    );
}

#[test]
fn the_palette_opens_on_command_k_and_jumps_to_a_note() {
    let mut screen = Screen::new("palette");
    let welcome = screen.note_named("Welcome");
    let roadmap = screen.note_named("Roadmap");
    screen.open(welcome);

    assert!(!screen.shell.chrome().palette.is_open());
    screen.key(KeyCode::Character('k'), Modifiers::COMMAND);
    screen.quiesce();
    assert!(screen.shell.chrome().palette.is_open());
    assert!(
        screen.has(crate::palette::PALETTE_LABEL),
        "{:?}",
        screen.labels()
    );
    assert!(screen.labels().iter().any(|l| l == "Roadmap"));

    // Running the command is what a click on the row does.
    crate::palette::run(
        screen.shell.store(),
        screen.shell.chrome(),
        &crate::palette::note_command_id(roadmap),
    );
    screen.quiesce();
    assert_eq!(screen.open_note(), Some(roadmap));
    assert!(!screen.shell.chrome().palette.is_open());
    assert!(screen.value(EDITOR_LABEL).starts_with("Roadmap"));
}

#[test]
fn command_n_makes_a_real_file_and_opens_it() {
    let mut screen = Screen::new("new");
    let before = screen.shell.store().library.peek_with(|l| l.notes().len());

    screen.key(KeyCode::Character('n'), Modifiers::COMMAND);
    screen.quiesce();

    let after = screen.shell.store().library.peek_with(|l| l.notes().len());
    assert_eq!(after, before + 1);
    let created = screen.note_named("Untitled note");
    assert_eq!(screen.open_note(), Some(created));
    assert!(
        screen.file_of(created).is_file(),
        "catatan baru harus jadi berkas sungguhan"
    );
}

#[test]
fn command_s_writes_without_waiting_for_the_debounce() {
    let mut screen = Screen::new("save");
    let note = screen.note_named("Welcome");
    screen.open(note);
    screen.focus_editor();
    screen.key(KeyCode::Named(NamedKey::End), Modifiers::COMMAND);
    screen.type_text(" NOW");
    assert_eq!(screen.shell.store().status(), SaveStatus::Pending(1));

    screen.key(KeyCode::Character('s'), Modifiers::COMMAND);
    screen.quiesce();

    assert_eq!(screen.shell.store().status(), SaveStatus::Saved);
    let on_disk = fs::read_to_string(screen.file_of(note)).expect("read");
    assert!(on_disk.contains("NOW"), "{on_disk}");
}

#[test]
fn the_auto_save_waits_for_the_typing_to_stop() {
    let mut screen = Screen::new("debounce");
    let note = screen.note_named("Welcome");
    screen.open(note);
    screen.focus_editor();
    screen.key(KeyCode::Named(NamedKey::End), Modifiers::COMMAND);
    screen.type_text(" DEBOUNCE");

    // Frames pass, but no time to speak of: nothing has been written.
    for _ in 0..5 {
        screen.frame();
    }
    let on_disk = fs::read_to_string(screen.file_of(note)).expect("read");
    assert!(!on_disk.contains("DEBOUNCE"), "terlalu cepat menyimpan");

    screen.wait(state::AUTOSAVE_DELAY + Duration::from_millis(50));
    let on_disk = fs::read_to_string(screen.file_of(note)).expect("read");
    assert!(on_disk.contains("DEBOUNCE"), "{on_disk}");
    assert_eq!(screen.shell.store().status(), SaveStatus::Saved);
}

#[test]
fn what_is_written_to_disk_is_markdown_a_human_would_recognise() {
    let mut screen = Screen::with("format", |root| {
        fs::write(root.join("Format.md"), "start\n").expect("write");
    });
    let note = screen.note_named("Format");
    screen.open(note);
    screen.focus_editor();
    screen.key(KeyCode::Named(NamedKey::End), Modifiers::COMMAND);
    screen.key(KeyCode::Named(NamedKey::Enter), Modifiers::NONE);
    screen.type_text("a heading");
    // ⌘⌥1 is the editor's own shortcut for "make this a heading".
    screen.key(
        KeyCode::Character('1'),
        Modifiers::COMMAND.union(Modifiers::ALT),
    );
    screen.key(KeyCode::Character('s'), Modifiers::COMMAND);
    screen.quiesce();

    let on_disk = fs::read_to_string(screen.file_of(note)).expect("read");
    assert!(
        on_disk.contains("# a heading"),
        "judul harus ditulis sebagai Markdown:\n{on_disk}"
    );
    // …and reading it back gives exactly the document that was on screen.
    assert_eq!(
        markdown::from_markdown(&on_disk),
        screen.document(note).expect("dokumen")
    );
}

#[test]
fn a_note_is_read_off_the_thread_that_draws() {
    // The load is a task: right after opening a note that has never been read,
    // the pane says so instead of blocking the frame.
    let mut screen = Screen::new("async");
    let roadmap = screen.note_named("Roadmap");

    screen.shell.store().open_note(roadmap);
    // One frame only — no `quiesce`, so the task cannot have landed yet.
    screen.shell.pump(screen.clock);
    screen.shell.ui.frame();
    assert!(
        screen.has(crate::editor::LOADING) || screen.has(EDITOR_LABEL),
        "panel harus menyebut pemuatan, bukan membeku: {:?}",
        screen.labels()
    );

    screen.quiesce();
    assert!(screen.value(EDITOR_LABEL).starts_with("Roadmap"));
}

#[test]
fn a_copy_made_here_keeps_its_styling_when_it_comes_back() {
    // The mirror image of the "paste from outside" test: text that *this*
    // application copied carries its inline styling back, because the rich
    // flavour beside it on the pasteboard describes the very same characters.
    let mut screen = Screen::with("copy", |root| {
        fs::write(root.join("Copy.md"), "**bold** and plain\n").expect("write");
    });
    let note = screen.note_named("Copy");
    screen.open(note);

    let before = screen.document(note).expect("dokumen");
    assert_eq!(
        bold_runs(&before),
        1,
        "prasyarat: catatan mulai dengan satu rentang tebal"
    );

    screen.focus_editor();
    // Select "bold" — four characters from the start of the note.
    screen.press(KeyCode::Named(NamedKey::Home), Modifiers::COMMAND);
    for _ in 0..4 {
        screen.press(KeyCode::Named(NamedKey::ArrowRight), Modifiers::SHIFT);
    }
    screen.key(KeyCode::Character('c'), Modifiers::COMMAND);
    screen.quiesce();

    // Paste it at the end of the note.
    screen.key(KeyCode::Named(NamedKey::End), Modifiers::COMMAND);
    screen.key(KeyCode::Character('v'), Modifiers::COMMAND);
    screen.quiesce();

    let after = screen.document(note).expect("dokumen");
    assert!(
        after.plain_text().ends_with("bold"),
        "tempelan harus mendarat di ujung: {:?}",
        after.plain_text()
    );
    assert_eq!(
        bold_runs(&after),
        2,
        "salinan dari dalam aplikasi harus membawa gaya inline-nya: {:?}",
        after.blocks()
    );
}

/// How many runs of bold text a document holds.
fn bold_runs(document: &Document) -> usize {
    document
        .blocks()
        .iter()
        .flat_map(|b| b.spans.iter())
        .filter(|s| s.style.marks.contains(silka_widgets::wysiwyg::Marks::BOLD))
        .count()
}

#[test]
fn clicking_a_note_in_the_outline_opens_it() {
    let mut screen = Screen::new("click");
    let welcome = screen.note_named("Welcome");
    screen.open(welcome);
    assert_eq!(screen.open_note(), Some(welcome));

    // "Journal" is a folder. Clicking it selects the row; → is what opens it,
    // exactly as in Finder — and the tree's own keyboard is what makes that
    // work without this application writing a line of it.
    screen.click_label("Journal");
    screen.key(KeyCode::Named(NamedKey::ArrowRight), Modifiers::NONE);
    screen.quiesce();
    assert!(
        screen.labels().iter().any(|l| l == "Reading"),
        "membuka folder harus memperlihatkan isinya: {:?}",
        screen.labels()
    );
    // A folder is not a note: selecting it must not have changed what is open.
    assert_eq!(screen.open_note(), Some(welcome));

    screen.click_label("Reading");
    screen.quiesce();
    assert_eq!(screen.open_note(), Some(screen.note_named("Reading")));
    assert!(screen.value(EDITOR_LABEL).starts_with("Reading"));
}

#[test]
fn the_application_settles_in_both_presets_and_both_appearances() {
    for preset in [Preset::Cupertino, Preset::Tailwind] {
        for appearance in [Appearance::Light, Appearance::Dark] {
            let theme = Theme::new(preset, appearance);
            let fonts = Fonts::new();
            install_fonts(&fonts);
            let scratch = Scratch::new("themes");
            store::seed(scratch.path()).expect("seed");
            let library = store::scan(scratch.path()).expect("scan");
            let mut shell =
                Shell::new(theme, library, Mode::InProcess).sized(VIEWPORT.width, VIEWPORT.height);

            let mut clock = Instant::now();
            let mut settled = false;
            for _ in 0..900 {
                if !shell.ui.tasks().is_idle() {
                    shell.ui.tasks().wait_for_idle();
                }
                clock += FRAME;
                shell.pump(clock);
                shell.ui.animate_at(clock, silka_widgets::advance);
                shell.ui.frame();
                if shell.ui.is_idle() && shell.ui.tasks().is_idle() {
                    settled = true;
                    break;
                }
            }
            assert!(settled, "{preset:?}/{appearance:?} tidak pernah diam");
            assert_eq!(shell.ui.scene().clear_color(), theme.color.background);
            assert!(shell.ui.access_tree().find_label(EDITOR_LABEL).is_some());
        }
    }
}

#[test]
fn a_shortcut_never_reaches_the_editor_as_a_character() {
    let mut screen = Screen::new("swallow");
    let note = screen.note_named("Welcome");
    screen.open(note);
    screen.focus_editor();
    let before = screen.document(note).expect("dokumen");

    for code in ['k', 's', 'n'] {
        screen.key(KeyCode::Character(code), Modifiers::COMMAND);
    }
    screen.quiesce();

    // ⌘N made a note, so the open one changed — but the note that was open must
    // not have gained the letters.
    let after = screen.document(note).expect("dokumen");
    assert_eq!(
        after.plain_text(),
        before.plain_text(),
        "pintasan aplikasi tidak boleh ikut diketikkan"
    );
}

#[test]
fn a_pending_shortcut_from_a_view_is_served_on_the_next_frame() {
    let mut screen = Screen::new("pending");
    let before = screen.shell.store().library.peek_with(|l| l.notes().len());
    screen.shell.chrome().pending.set(Some(Shortcut::New));
    screen.quiesce();
    assert_eq!(
        screen.shell.store().library.peek_with(|l| l.notes().len()),
        before + 1
    );
    assert_eq!(screen.shell.chrome().pending.peek(), None);
}

#[test]
fn an_empty_notes_directory_still_opens() {
    let screen = Screen::with("empty", |_| {});
    assert!(screen.open_note().is_none());
    assert!(
        screen.has(crate::editor::NOTHING_OPEN),
        "{:?}",
        screen.labels()
    );
    assert!(screen.has(sidebar::OUTLINE_LABEL));
}

#[test]
fn a_document_built_by_hand_survives_the_whole_round_trip() {
    // Not through the window: the file format on its own, over the document the
    // editor would produce. Belongs here rather than in `markdown` because it
    // is the *application's* promise — what you type is what is on disk.
    let document = Document::from_blocks(vec![
        Block::plain(BlockKind::Heading1, "Title"),
        Block::plain(BlockKind::Paragraph, "A paragraph."),
        Block::empty(),
        Block::plain(BlockKind::Bullet, "one"),
        Block::plain(BlockKind::Bullet, "two"),
        Block::plain(BlockKind::Code, "cargo test -p silka-notes"),
    ]);
    let scratch = Scratch::new("format-only");
    let path = scratch.path().join("Round.md");
    store::save(&path, &markdown::to_markdown(&document)).expect("save");
    let read_back = markdown::from_markdown(&store::load(&path).expect("load"));
    assert_eq!(read_back, document);
}
