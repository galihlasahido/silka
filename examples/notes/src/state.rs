//! The application's state, and the background work that keeps it and the disk
//! agreeing.
//!
//! ## Five signals, not one
//!
//! The obvious design is one `Signal<AppState>`. It is also the wrong one here,
//! and the reason is the editor: a signal marks **every component that read it**
//! dirty, so a single blob of state means every keystroke rebuilds the sidebar,
//! the status bar and the editor together. With a thousand-paragraph note open
//! that is a thousand-block clone and a full re-flatten of the outline per
//! character typed.
//!
//! So the state is split by *who reads it*:
//!
//! | Signal | Written when | Read by |
//! |---|---|---|
//! | [`Store::library`] | the directory is scanned | the sidebar, the palette |
//! | [`Store::open`] | a note is chosen | everything |
//! | [`Store::docs`] | **every keystroke** | the status bar |
//! | [`Store::epoch`] | a document is replaced *from outside the editor* | the editor |
//! | [`Store::query`] | the search field | the sidebar |
//! | [`Store::index`] | a note is saved, or the initial scan finishes | the sidebar |
//!
//! [`Store::epoch`] is the load-bearing one. The editor body owns its document
//! while it is being typed into — that is `wysiwyg`'s contract — so the editor
//! must **not** be rebuilt by ordinary edits, and must be rebuilt when a note is
//! opened or a load lands. One counter says exactly that and nothing else.
//!
//! ## Nothing touches the disk on the UI thread
//!
//! Reading a note, writing a note and building the search index all run through
//! [`silka_core::task::Tasks`]: the work is `Send` and knows nothing about
//! signals, the continuation is not `Send` and does nothing else
//! (REKOMENDASI §9.6). What is left on the UI thread is [`pump`], which decides
//! *whether* to start any of that, and is a pure decision over counters.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use silka_core::signals::{Runtime, Signal};
use silka_core::task::Tasks;
use silka_widgets::wysiwyg::Document;
use silka_widgets::TreeKey;

use crate::markdown;
use crate::search::{self, Hit};
use crate::stats::{self, Stats};
use crate::store::{self, Library};

/// How long the typing has to pause before a note is written to disk.
///
/// Long enough that a burst of typing is one write rather than forty, short
/// enough that "did it save?" is never a question anyone asks. A save is also
/// forced — regardless of this — whenever the open note changes or ⌘S is
/// pressed, which is the part that actually protects the writing.
pub const AUTOSAVE_DELAY: Duration = Duration::from_millis(600);

// ---------------------------------------------------------------------------
// Buffers
// ---------------------------------------------------------------------------

/// What is known about one note's contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Loaded {
    /// The read has been started and has not answered yet.
    Loading,
    /// The document, live: this is the copy the editor is typing into.
    Ready(Document),
    /// The read failed, with something to show the user.
    Failed(String),
}

/// One open (or recently open) note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Buffer {
    state: Loaded,
    /// Bumped by every edit. The difference between this and [`Buffer::saved`]
    /// **is** the definition of "unsaved changes" — a boolean would be wrong
    /// the moment an edit lands while a save is in flight.
    edits: u64,
    /// The edit count that is known to be on disk.
    saved: u64,
    /// The edit count currently being written, when a write is in flight.
    saving: Option<u64>,
    /// When the debounce started — stamped by [`pump`], never by the edit.
    ///
    /// The edit itself has no clock it can trust: `on_change` runs inside event
    /// dispatch, where the only time available is `Instant::now()` — the real
    /// one. A test drives a **fake** clock, and mixing the two means a debounce
    /// that either fires immediately or never fires at all. So the edit only
    /// bumps a counter, and the pump — which is handed the frame's clock —
    /// stamps the time.
    edited_at: Option<Instant>,
    /// The edit count the pump has already stamped.
    seen: u64,
    /// True while the file is being read.
    ///
    /// Separate from `state` on purpose: "the document is not here yet" and "a
    /// read is in flight" are different facts, and collapsing them is how a
    /// failed read turns into a request that is retried on every single frame.
    reading: bool,
}

impl Buffer {
    /// A buffer waiting for its file to be read.
    fn loading() -> Self {
        Self {
            state: Loaded::Loading,
            edits: 0,
            saved: 0,
            saving: None,
            edited_at: None,
            seen: 0,
            reading: false,
        }
    }

    /// The document, when there is one.
    pub fn document(&self) -> Option<&Document> {
        match &self.state {
            Loaded::Ready(d) => Some(d),
            _ => None,
        }
    }

    /// True while the document is not here yet.
    pub fn is_loading(&self) -> bool {
        matches!(self.state, Loaded::Loading)
    }

    /// The read error, when the file could not be opened.
    pub fn error(&self) -> Option<&str> {
        match &self.state {
            Loaded::Failed(e) => Some(e.as_str()),
            _ => None,
        }
    }

    /// True when this note holds writing that is not on disk yet.
    pub fn is_dirty(&self) -> bool {
        self.edits != self.saved
    }

    /// True while a write is in flight.
    pub fn is_saving(&self) -> bool {
        self.saving.is_some()
    }
}

/// Every buffer, plus what has happened to them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Docs {
    buffers: BTreeMap<TreeKey, Buffer>,
    /// Completed writes — what a "saved 3 times" debug readout would show, and
    /// what proves a save really happened rather than merely being planned.
    saves: usize,
    /// The last thing that went wrong while saving.
    error: Option<String>,
}

impl Docs {
    /// One buffer.
    pub fn buffer(&self, note: TreeKey) -> Option<&Buffer> {
        self.buffers.get(&note)
    }

    /// One note's document.
    pub fn document(&self, note: TreeKey) -> Option<&Document> {
        self.buffers.get(&note).and_then(Buffer::document)
    }

    /// How many notes hold unsaved writing.
    pub fn dirty_count(&self) -> usize {
        self.buffers.values().filter(|b| b.is_dirty()).count()
    }

    /// True when a write is in flight anywhere.
    pub fn is_saving(&self) -> bool {
        self.buffers.values().any(Buffer::is_saving)
    }

    /// The last save error, if any.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

/// What the status bar says about the disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveStatus {
    /// Everything the user has written is on disk.
    Saved,
    /// There are unsaved changes; the write has not started yet.
    Pending(usize),
    /// A write is in flight.
    Saving,
    /// The last write failed.
    Failed(String),
}

impl SaveStatus {
    /// The line shown in the status bar, and the a11y name of the indicator.
    pub fn label(&self) -> String {
        match self {
            SaveStatus::Saved => "All changes saved".to_string(),
            SaveStatus::Pending(1) => "Unsaved changes".to_string(),
            SaveStatus::Pending(n) => format!("Unsaved changes in {n} notes"),
            SaveStatus::Saving => "Saving…".to_string(),
            SaveStatus::Failed(e) => format!("Could not save: {e}"),
        }
    }
}

/// The searchable text of every note.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Index {
    text: BTreeMap<TreeKey, String>,
    requested: bool,
    ready: bool,
}

impl Index {
    /// True once the initial scan has answered.
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// One note's plain text.
    pub fn text_of(&self, note: TreeKey) -> &str {
        self.text.get(&note).map(String::as_str).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// The store
// ---------------------------------------------------------------------------

/// Every signal the application shares, in one `Copy` handle.
///
/// A handle rather than a struct of values: each field is a [`Signal`], so
/// cloning this into a closure clones five identifiers and no data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Store {
    /// The notes directory as it was last scanned.
    pub library: Signal<Library>,
    /// The note being edited.
    pub open: Signal<Option<TreeKey>>,
    /// The buffers.
    pub docs: Signal<Docs>,
    /// Bumped whenever a document is replaced from outside the editor.
    pub epoch: Signal<u64>,
    /// What is typed into the search field.
    pub query: Signal<String>,
    /// The search index.
    pub index: Signal<Index>,
}

impl Store {
    /// Create every signal on `runtime`, opening `library`'s first note.
    pub fn install(runtime: &Runtime, library: Library) -> Self {
        let first = library.first_note().map(|n| n.id);
        let store = Self {
            library: runtime.signal(library),
            open: runtime.signal(first),
            docs: runtime.signal(Docs::default()),
            epoch: runtime.signal(0),
            query: runtime.signal(String::new()),
            index: runtime.signal(Index::default()),
        };
        if let Some(id) = first {
            store.ensure_buffer(id);
        }
        store
    }

    /// Make sure a buffer exists for `note`, so [`pump`] knows to read it.
    fn ensure_buffer(&self, note: TreeKey) {
        self.docs.update(|d| {
            d.buffers.entry(note).or_insert_with(Buffer::loading);
        });
    }

    /// Open a note.
    ///
    /// Does **not** touch the note that was open: its buffer stays exactly as
    /// it is, unsaved edits and all. That is the whole answer to "switching
    /// notes must not lose writing" — there is nowhere for the writing to go.
    pub fn open_note(&self, note: TreeKey) {
        if self.open.peek() == Some(note) {
            return;
        }
        self.ensure_buffer(note);
        self.open.set(Some(note));
        // The editor is keyed on the open note and rebuilt by this counter;
        // without the bump it would go on showing the previous document.
        self.epoch.update(|e| *e += 1);
    }

    /// Record what the editor produced.
    ///
    /// Deliberately does not bump [`Store::epoch`]: the editor already holds
    /// this document — rebuilding it here is how an editor throws the caret
    /// back to the start of the note on every keystroke.
    pub fn edit(&self, note: TreeKey, document: Document) {
        self.docs.update(|d| {
            let buffer = d.buffers.entry(note).or_insert_with(Buffer::loading);
            if buffer.document() == Some(&document) {
                return;
            }
            buffer.state = Loaded::Ready(document);
            buffer.edits += 1;
        });
    }

    /// Replace a document from outside the editor (a load, an undo of a whole
    /// note), and make the editor pick it up.
    pub fn replace(&self, note: TreeKey, state: Loaded) {
        self.docs.update(|d| {
            let buffer = d.buffers.entry(note).or_insert_with(Buffer::loading);
            buffer.state = state;
            buffer.reading = false;
        });
        self.epoch.update(|e| *e += 1);
    }

    /// The open note's word count.
    pub fn open_stats(&self) -> Stats {
        self.open
            .peek()
            .and_then(|note| self.docs.with(|d| d.document(note).map(stats::count)))
            .unwrap_or_default()
    }

    /// What the status bar should say.
    pub fn status(&self) -> SaveStatus {
        self.docs.with(|d| {
            if let Some(e) = d.error() {
                return SaveStatus::Failed(e.to_string());
            }
            if d.is_saving() {
                return SaveStatus::Saving;
            }
            match d.dirty_count() {
                0 => SaveStatus::Saved,
                n => SaveStatus::Pending(n),
            }
        })
    }

    /// The search results for the current query.
    ///
    /// The live buffer of a note that is open wins over the index, so a word
    /// just typed is findable before it has been written to disk.
    pub fn results(&self) -> Vec<Hit> {
        let query = self.query.with(String::clone);
        if query.trim().is_empty() {
            return Vec::new();
        }
        let bodies: Vec<(TreeKey, String, String)> = self.library.with(|library| {
            library
                .notes()
                .iter()
                .map(|note| {
                    // `peek_with`, not `with`: the sidebar calls this, and
                    // subscribing it to the buffers would rebuild the whole
                    // outline on every keystroke in the editor. Search follows
                    // the index, which moves when the disk does.
                    let live = self
                        .docs
                        .peek_with(|d| d.document(note.id).map(Document::plain_text));
                    let body =
                        live.unwrap_or_else(|| self.index.with(|i| i.text_of(note.id).to_string()));
                    (note.id, note.title.clone(), body)
                })
                .collect()
        });
        search::search(
            bodies
                .iter()
                .map(|(id, title, body)| (*id, title.as_str(), body.as_str())),
            &query,
        )
    }
}

// ---------------------------------------------------------------------------
// The pump
// ---------------------------------------------------------------------------

/// One unit of background work, decided on the UI thread and run off it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Job {
    /// Read one note.
    Load(TreeKey, PathBuf),
    /// Write one note, at a known edit count.
    Save(TreeKey, PathBuf, u64, String),
    /// Read every note into the search index.
    Index(Vec<(TreeKey, PathBuf)>),
}

/// Start whatever background work the current state calls for.
///
/// Called once per frame, before the rebuild. Returns how many jobs it started,
/// which is what the tests assert on — "did typing schedule exactly one save?"
/// is a question with a number for an answer.
///
/// `force` skips the debounce: that is ⌘S, and it is also what happens when the
/// open note changes.
pub fn pump(store: &Store, tasks: &Tasks, now: Instant, force: bool) -> usize {
    stamp(store, now);
    let jobs = plan(store, now, force);
    for job in &jobs {
        mark_started(store, job);
    }
    for job in jobs.iter().cloned() {
        spawn(store, tasks, job);
    }
    jobs.len()
}

/// Save everything that is dirty, right now.
pub fn flush(store: &Store, tasks: &Tasks, now: Instant) -> usize {
    pump(store, tasks, now, true)
}

/// Restart the debounce of every buffer that has been edited since the last
/// frame.
///
/// This is where the edit gets its timestamp, on the frame's clock — see
/// [`Buffer::edited_at`] for why it cannot be taken where the edit happens.
fn stamp(store: &Store, now: Instant) {
    let unstamped = store
        .docs
        .peek_with(|d| d.buffers.values().any(|b| b.seen != b.edits));
    if !unstamped {
        return;
    }
    store.docs.update(|d| {
        for buffer in d.buffers.values_mut() {
            if buffer.seen != buffer.edits {
                buffer.seen = buffer.edits;
                buffer.edited_at = Some(now);
            }
        }
    });
}

/// Decide what to do — a pure read of the signals, so it can be tested without
/// a task runner.
fn plan(store: &Store, now: Instant, force: bool) -> Vec<Job> {
    let mut jobs = Vec::new();

    if !store.index.peek_with(|i| i.requested) {
        let files = store.library.peek_with(|l| {
            l.notes()
                .iter()
                .map(|n| (n.id, n.file.clone()))
                .collect::<Vec<_>>()
        });
        jobs.push(Job::Index(files));
    }

    let paths: BTreeMap<TreeKey, PathBuf> = store
        .library
        .peek_with(|l| l.notes().iter().map(|n| (n.id, n.file.clone())).collect());

    store.docs.peek_with(|docs| {
        for (id, buffer) in &docs.buffers {
            let Some(path) = paths.get(id) else {
                // The file is gone from the directory; there is nothing to read
                // and nowhere to write. The buffer stays, so the writing is not
                // lost — it simply has no home until the next scan finds one.
                continue;
            };
            if buffer.is_loading() {
                if !buffer.reading {
                    jobs.push(Job::Load(*id, path.clone()));
                }
                continue;
            }
            if !buffer.is_dirty() || buffer.is_saving() {
                continue;
            }
            let due = force
                || buffer
                    .edited_at
                    .is_some_and(|at| now.duration_since(at) >= AUTOSAVE_DELAY);
            if !due {
                continue;
            }
            if let Some(document) = buffer.document() {
                jobs.push(Job::Save(
                    *id,
                    path.clone(),
                    buffer.edits,
                    markdown::to_markdown(document),
                ));
            }
        }
    });

    jobs
}

/// Record that a job is in flight, so the next frame does not start it again.
fn mark_started(store: &Store, job: &Job) {
    match job {
        Job::Index(_) => store.index.update(|i| i.requested = true),
        Job::Load(id, _) => store.docs.update(|d| {
            if let Some(b) = d.buffers.get_mut(id) {
                b.reading = true;
            }
        }),
        Job::Save(id, _, edits, _) => store.docs.update(|d| {
            if let Some(b) = d.buffers.get_mut(id) {
                b.saving = Some(*edits);
            }
        }),
    }
}

/// Hand a job to a background thread.
fn spawn(store: &Store, tasks: &Tasks, job: Job) {
    let store = *store;
    match job {
        Job::Index(files) => {
            tasks.spawn_blocking(
                move |cancel| {
                    let mut out: Vec<(TreeKey, String)> = Vec::with_capacity(files.len());
                    for (id, path) in files {
                        if cancel.is_cancelled() {
                            break;
                        }
                        // Parsing here rather than on the UI thread is the
                        // point: the index of a directory full of long notes
                        // is exactly the work that must not land in a frame.
                        let text = store::load(&path).unwrap_or_default();
                        out.push((id, markdown::from_markdown(&text).plain_text()));
                    }
                    out
                },
                move |entries| {
                    store.index.update(|index| {
                        for (id, text) in entries {
                            index.text.insert(id, text);
                        }
                        index.ready = true;
                    });
                },
            );
        }
        Job::Load(id, path) => {
            tasks.spawn_blocking(
                move |_| store::load(&path).map_err(|e| e.to_string()),
                move |result| {
                    let state = match result {
                        Ok(text) => Loaded::Ready(markdown::from_markdown(&text)),
                        Err(e) => Loaded::Failed(e),
                    };
                    store.replace(id, state);
                },
            );
        }
        Job::Save(id, path, edits, text) => {
            let indexed = markdown::from_markdown(&text).plain_text();
            tasks.spawn_blocking(
                move |_| store::save(&path, &text).map_err(|e| e.to_string()),
                move |result| {
                    store.docs.update(|d| {
                        let Some(buffer) = d.buffers.get_mut(&id) else {
                            return;
                        };
                        buffer.saving = None;
                        match result {
                            Ok(()) => {
                                // `max`, not `=`: an edit that arrived while
                                // the write was in flight has already moved
                                // `edits` past this, and it is still unsaved.
                                buffer.saved = buffer.saved.max(edits);
                                d.saves += 1;
                                d.error = None;
                            }
                            Err(e) => d.error = Some(e),
                        }
                    });
                    // The file on disk is what search reads, so the index only
                    // moves when the disk does.
                    store.index.update(|i| {
                        i.text.insert(id, indexed);
                    });
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Library;

    /// A store on a bare runtime — no window, no tasks.
    fn store(runtime: &Runtime) -> Store {
        Store::install(runtime, Library::empty("/nowhere"))
    }

    #[test]
    fn an_edit_is_unsaved_until_a_write_lands() {
        let rt = Runtime::new();
        let s = store(&rt);

        s.edit(7, Document::from_plain("hello"));
        assert_eq!(s.status(), SaveStatus::Pending(1));
        assert_eq!(s.docs.peek_with(Docs::dirty_count), 1);

        // The buffer answers the same question the status bar does.
        s.docs.update(|d| {
            let b = d.buffers.get_mut(&7).expect("buffer");
            b.saved = b.edits;
        });
        assert_eq!(s.status(), SaveStatus::Saved);
    }

    #[test]
    fn an_edit_during_a_save_stays_unsaved() {
        // The classic auto-save bug: the write of revision 3 lands, the flag is
        // cleared, and revision 4 — typed while the disk was busy — is lost.
        let rt = Runtime::new();
        let s = store(&rt);

        s.edit(7, Document::from_plain("one"));
        let in_flight = s.docs.peek_with(|d| d.buffer(7).expect("buffer").edits);
        s.docs
            .update(|d| d.buffers.get_mut(&7).expect("buffer").saving = Some(in_flight));

        s.edit(7, Document::from_plain("one two"));
        s.docs.update(|d| {
            let b = d.buffers.get_mut(&7).expect("buffer");
            b.saving = None;
            b.saved = b.saved.max(in_flight);
        });

        assert!(s
            .docs
            .peek_with(|d| d.buffer(7).expect("buffer").is_dirty()));
    }

    #[test]
    fn writing_the_same_document_again_is_not_an_edit() {
        let rt = Runtime::new();
        let s = store(&rt);
        s.edit(7, Document::from_plain("hello"));
        let edits = s.docs.peek_with(|d| d.buffer(7).expect("buffer").edits);
        s.edit(7, Document::from_plain("hello"));
        assert_eq!(
            s.docs.peek_with(|d| d.buffer(7).expect("buffer").edits),
            edits
        );
    }

    #[test]
    fn opening_the_note_that_is_already_open_does_not_bump_the_epoch() {
        let rt = Runtime::new();
        let s = store(&rt);
        s.open_note(3);
        let epoch = s.epoch.peek();
        s.open_note(3);
        assert_eq!(s.epoch.peek(), epoch);
        s.open_note(4);
        assert_eq!(s.epoch.peek(), epoch + 1);
    }

    #[test]
    fn the_debounce_is_what_decides_when_a_save_is_planned() {
        let rt = Runtime::new();
        let s = store(&rt);
        // The library is empty, so give the note a path the plan can find.
        s.library.update(|l| *l = Library::empty("/nowhere"));
        let start = Instant::now();
        s.index.update(|i| i.requested = true);

        s.edit(7, Document::from_plain("hello"));
        // No path for note 7 in an empty library: nothing can be planned, and
        // that must not be a panic.
        assert!(plan(&s, start + AUTOSAVE_DELAY, false).is_empty());
    }

    #[test]
    fn the_status_line_reads_like_a_sentence() {
        assert_eq!(SaveStatus::Saved.label(), "All changes saved");
        assert_eq!(SaveStatus::Pending(1).label(), "Unsaved changes");
        assert_eq!(SaveStatus::Pending(3).label(), "Unsaved changes in 3 notes");
        assert_eq!(SaveStatus::Saving.label(), "Saving…");
        assert!(SaveStatus::Failed("disk full".into())
            .label()
            .contains("disk full"));
    }
}
