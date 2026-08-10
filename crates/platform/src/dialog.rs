//! Native file and message dialogs (INTEGRASI-NATIVE §2).
//!
//! `rfd` lives only here — on Linux it goes through the XDG portal, which is
//! what makes a Flatpak build able to open files outside its sandbox at all.
//! Everything above this module speaks in `PathBuf`, `String`, and the enums
//! below.
//!
//! ```no_run
//! use silka_platform::dialog::{file_dialog, message, MessageAnswer};
//!
//! if let Some(path) = file_dialog()
//!     .title("Buka dokumen")
//!     .filter("Teks", &["txt", "md"])
//!     .pick_file()
//! {
//!     println!("{}", path.display());
//! }
//!
//! let jawab = message("Simpan?").body("Perubahan belum disimpan.").ask();
//! assert!(matches!(jawab, MessageAnswer::Yes | MessageAnswer::No));
//! ```
//!
//! ## Always give the dialog a parent
//!
//! [`FileDialog::parent`] and [`MessageDialog::parent`] are what turn a dialog
//! into a **sheet** on macOS and a properly owned modal on Windows. Without a
//! parent the dialog is an independent window: it can end up behind the
//! application, and on macOS it looks nothing like a native document dialog.
//! The builders take the parent by reference and carry its lifetime, so a
//! dialog can never outlive the window it is attached to.

use std::path::{Path, PathBuf};

use winit::window::Window;

/// Extensions accepted by one entry of a file dialog's format list.
///
/// ```
/// use silka_platform::dialog::file_dialog;
///
/// // Callers write ".csv" about as often as "csv"; both are accepted and
/// // normalised, because a stray dot silently filters everything out instead
/// // of erroring — the worst kind of bug to chase.
/// let dialog = file_dialog().filter("Spreadsheet", &[".CSV", "xlsx"]);
///
/// let filter = &dialog.filters()[0];
/// assert_eq!(filter.name(), "Spreadsheet");
/// assert_eq!(filter.extensions(), ["csv", "xlsx"]);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFilter {
    name: String,
    extensions: Vec<String>,
}

impl FileFilter {
    /// The label shown in the dialog's format popup.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The extensions, always without a leading dot.
    pub fn extensions(&self) -> &[String] {
        &self.extensions
    }
}

/// Normalise one extension.
///
/// Users of the API write `".txt"` about as often as `"txt"`, and every native
/// dialog wants the second form; a leading dot silently filters everything out
/// instead of erroring, which is the worst kind of bug to chase.
fn rapikan_ekstensi(ext: &str) -> String {
    ext.trim().trim_start_matches('.').to_ascii_lowercase()
}

/// A file dialog, built by method chaining.
///
/// The lifetime belongs to [`FileDialog::parent`]; without a parent it is
/// unconstrained.
///
/// ```
/// use silka_platform::dialog::file_dialog;
///
/// let dialog = file_dialog()
///     .title("Import transactions")
///     .file_name("transactions.csv")
///     .filter("Comma separated", &["csv"]);
///
/// assert_eq!(dialog.filters().len(), 1);
/// // Without `.parent(window)` the dialog is app-modal rather than a sheet.
/// assert!(!dialog.has_parent());
/// ```
///
/// Attaching a parent makes it a window sheet on macOS, and the borrow is what
/// guarantees the dialog can never outlive the window it hangs from. The
/// blocking calls are [`FileDialog::pick_file`], `pick_files`, `pick_folder`
/// and `save_file`.
#[derive(Debug, Clone)]
pub struct FileDialog<'a> {
    title: Option<String>,
    directory: Option<PathBuf>,
    file_name: Option<String>,
    filters: Vec<FileFilter>,
    parent: Option<&'a Window>,
}

/// Create a file dialog.
///
/// The dialog is the OS's own — on Linux it goes through the XDG portal, so it
/// works inside a Flatpak sandbox as well as outside one.
///
/// ```
/// use silka_platform::file_dialog;
///
/// // Built here, shown by `pick_file`/`pick_files`/`pick_folder`, each of
/// // which blocks and returns `None` when the user cancels.
/// let open = file_dialog()
///     .title("Open document")
///     .directory("/tmp")
///     .filter("Documents", &["md", "txt"])
///     .filter("Images", &["png", "jpg"]);
///
/// assert_eq!(open.filters().len(), 2);
/// assert!(!open.has_parent()); // app-modal rather than window-modal
///
/// // A save dialog is the same builder with a suggested name.
/// let save = file_dialog().file_name("Untitled.md");
/// # let _ = save;
/// ```
pub fn file_dialog<'a>() -> FileDialog<'a> {
    FileDialog {
        title: None,
        directory: None,
        file_name: None,
        filters: Vec::new(),
        parent: None,
    }
}

impl<'a> FileDialog<'a> {
    /// The dialog title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Where the dialog opens.
    pub fn directory(mut self, directory: impl AsRef<Path>) -> Self {
        self.directory = Some(directory.as_ref().to_path_buf());
        self
    }

    /// The name pre-filled in a save dialog.
    pub fn file_name(mut self, name: impl Into<String>) -> Self {
        self.file_name = Some(name.into());
        self
    }

    /// Add a format filter. Extensions may be written with or without a dot.
    pub fn filter(mut self, name: impl Into<String>, extensions: &[&str]) -> Self {
        self.filters.push(FileFilter {
            name: name.into(),
            extensions: extensions.iter().map(|e| rapikan_ekstensi(e)).collect(),
        });
        self
    }

    /// Attach the dialog to a window — a sheet on macOS, an owned modal
    /// elsewhere.
    pub fn parent(mut self, window: &'a Window) -> Self {
        self.parent = Some(window);
        self
    }

    /// The filters, in order.
    pub fn filters(&self) -> &[FileFilter] {
        &self.filters
    }

    /// Whether a parent window was given.
    pub fn has_parent(&self) -> bool {
        self.parent.is_some()
    }

    fn ke_rfd(&self) -> rfd::FileDialog {
        let mut d = rfd::FileDialog::new();
        if let Some(t) = &self.title {
            d = d.set_title(t);
        }
        if let Some(dir) = &self.directory {
            d = d.set_directory(dir);
        }
        if let Some(n) = &self.file_name {
            d = d.set_file_name(n.clone());
        }
        for f in &self.filters {
            d = d.add_filter(f.name.clone(), &f.extensions);
        }
        if let Some(w) = self.parent {
            d = d.set_parent(w);
        }
        d
    }

    /// Ask for one existing file. `None` means the user cancelled.
    pub fn pick_file(self) -> Option<PathBuf> {
        self.ke_rfd().pick_file()
    }

    /// Ask for several existing files.
    pub fn pick_files(self) -> Option<Vec<PathBuf>> {
        self.ke_rfd().pick_files()
    }

    /// Ask for a folder.
    pub fn pick_folder(self) -> Option<PathBuf> {
        self.ke_rfd().pick_folder()
    }

    /// Ask where to save.
    pub fn save_file(self) -> Option<PathBuf> {
        self.ke_rfd().save_file()
    }
}

/// How serious a message is — it picks the icon and, on some systems, the
/// sound.
///
/// ```
/// use silka_platform::dialog::MessageLevel;
///
/// // Information is the default: a dialog that shouts by default trains
/// // users to dismiss it without reading.
/// assert_eq!(MessageLevel::default(), MessageLevel::Info);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MessageLevel {
    /// Ordinary information.
    #[default]
    Info,
    /// Something the user should look at.
    Warning,
    /// Something went wrong.
    Error,
}

/// Which buttons a message dialog offers.
///
/// ```
/// use silka_platform::dialog::{message, MessageButtons};
///
/// assert_eq!(MessageButtons::default(), MessageButtons::Ok);
///
/// let confirm = message("Discard changes?")
///     .body("Your edits will be lost.")
///     .buttons(MessageButtons::YesNo);
/// assert_eq!(confirm.button_set(), MessageButtons::YesNo);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MessageButtons {
    /// A single OK.
    #[default]
    Ok,
    /// OK and Cancel.
    OkCancel,
    /// Yes and No.
    YesNo,
    /// Yes, No, and Cancel.
    YesNoCancel,
}

/// What the user chose.
///
/// Escape reports [`MessageAnswer::Cancel`], so dismissal and refusal are the
/// same answer — which is what a user means by pressing it.
///
/// ```
/// use silka_platform::dialog::MessageAnswer;
///
/// fn should_discard(answer: MessageAnswer) -> bool {
///     matches!(answer, MessageAnswer::Yes)
/// }
///
/// assert!(should_discard(MessageAnswer::Yes));
/// assert!(!should_discard(MessageAnswer::Cancel));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageAnswer {
    /// The affirmative button.
    Yes,
    /// The negative button.
    No,
    /// OK.
    Ok,
    /// Cancel — also what a dialog dismissed with Escape reports.
    Cancel,
}

/// A message dialog, built by method chaining.
///
/// ```
/// use silka_platform::dialog::{message, MessageButtons, MessageLevel};
///
/// let dialog = message("Could not save")
///     .body("The file is on a volume that is no longer mounted.")
///     .level(MessageLevel::Error)
///     .buttons(MessageButtons::Ok);
///
/// assert_eq!(dialog.button_set(), MessageButtons::Ok);
/// ```
///
/// [`MessageDialog::ask`] blocks and returns the answer;
/// [`MessageDialog::show`] just presents it. This is the **OS's own** dialog —
/// for one drawn inside the window, reach for `silka_widgets::dialog`.
#[derive(Debug, Clone)]
pub struct MessageDialog<'a> {
    title: String,
    body: String,
    level: MessageLevel,
    buttons: MessageButtons,
    parent: Option<&'a Window>,
}

/// Create a message dialog with the given title.
///
/// This is the **OS's** alert, not the in-app `silka_widgets::dialog`. Reach
/// for it when the message must survive the application being busy — a startup
/// failure, a file that cannot be written — and for the in-app one otherwise.
///
/// ```
/// use silka_platform::{message, MessageButtons, MessageLevel};
///
/// // A question: `ask()` blocks and returns which button was pressed.
/// let confirm = message("Discard changes?")
///     .body("This cannot be undone.")
///     .level(MessageLevel::Warning)
///     .buttons(MessageButtons::YesNo);
/// assert_eq!(confirm.button_set(), MessageButtons::YesNo);
///
/// // A statement: `show()` just puts it on screen.
/// let note = message("Export finished").body("42 files written.");
/// assert_eq!(note.button_set(), MessageButtons::Ok);
/// ```
pub fn message<'a>(title: impl Into<String>) -> MessageDialog<'a> {
    MessageDialog {
        title: title.into(),
        body: String::new(),
        level: MessageLevel::Info,
        buttons: MessageButtons::Ok,
        parent: None,
    }
}

impl<'a> MessageDialog<'a> {
    /// The explanatory text below the title.
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    /// How serious the message is.
    pub fn level(mut self, level: MessageLevel) -> Self {
        self.level = level;
        self
    }

    /// Which buttons to offer.
    pub fn buttons(mut self, buttons: MessageButtons) -> Self {
        self.buttons = buttons;
        self
    }

    /// Attach the dialog to a window.
    pub fn parent(mut self, window: &'a Window) -> Self {
        self.parent = Some(window);
        self
    }

    /// The buttons this dialog will show.
    pub fn button_set(&self) -> MessageButtons {
        self.buttons
    }

    fn ke_rfd(&self) -> rfd::MessageDialog {
        let mut d = rfd::MessageDialog::new()
            .set_title(self.title.clone())
            .set_description(self.body.clone())
            .set_level(rfd_level(self.level))
            .set_buttons(rfd_buttons(self.buttons));
        if let Some(w) = self.parent {
            d = d.set_parent(w);
        }
        d
    }

    /// Show the dialog and wait for the answer.
    ///
    /// Blocking on purpose: a modal dialog *is* a blocking question, and
    /// pretending otherwise leaves the caller writing a state machine for a
    /// decision the user has already made.
    pub fn ask(self) -> MessageAnswer {
        jawaban_dari_rfd(self.ke_rfd().show())
    }

    /// Show the dialog purely to inform, ignoring the answer.
    pub fn show(self) {
        let _ = self.ask();
    }
}

fn rfd_level(level: MessageLevel) -> rfd::MessageLevel {
    match level {
        MessageLevel::Info => rfd::MessageLevel::Info,
        MessageLevel::Warning => rfd::MessageLevel::Warning,
        MessageLevel::Error => rfd::MessageLevel::Error,
    }
}

fn rfd_buttons(buttons: MessageButtons) -> rfd::MessageButtons {
    match buttons {
        MessageButtons::Ok => rfd::MessageButtons::Ok,
        MessageButtons::OkCancel => rfd::MessageButtons::OkCancel,
        MessageButtons::YesNo => rfd::MessageButtons::YesNo,
        MessageButtons::YesNoCancel => rfd::MessageButtons::YesNoCancel,
    }
}

/// Translate rfd's answer into ours.
///
/// A custom button is not something this API can produce, so if one ever comes
/// back it is treated as a cancel: the safe reading of "the user pressed
/// something we do not understand" is *do not proceed*.
fn jawaban_dari_rfd(result: rfd::MessageDialogResult) -> MessageAnswer {
    match result {
        rfd::MessageDialogResult::Yes => MessageAnswer::Yes,
        rfd::MessageDialogResult::No => MessageAnswer::No,
        rfd::MessageDialogResult::Ok => MessageAnswer::Ok,
        rfd::MessageDialogResult::Cancel => MessageAnswer::Cancel,
        rfd::MessageDialogResult::Custom(_) => MessageAnswer::Cancel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ekstensi_dinormalkan_tanpa_titik() {
        // A leading dot makes native dialogs match nothing at all — silently.
        assert_eq!(rapikan_ekstensi(".TXT"), "txt");
        assert_eq!(rapikan_ekstensi(" md "), "md");
        assert_eq!(rapikan_ekstensi("png"), "png");
    }

    #[test]
    fn filter_menyimpan_nama_dan_ekstensi_bersih() {
        let d = file_dialog().filter("Gambar", &[".PNG", "jpg"]);
        assert_eq!(d.filters().len(), 1);
        assert_eq!(d.filters()[0].name(), "Gambar");
        assert_eq!(d.filters()[0].extensions(), ["png", "jpg"]);
    }

    #[test]
    fn dialog_bawaan_tanpa_induk_dan_tanpa_filter() {
        let d = file_dialog();
        assert!(!d.has_parent());
        assert!(d.filters().is_empty());
        assert!(d.title.is_none());
        assert!(d.directory.is_none());
    }

    #[test]
    fn chaining_mengisi_hanya_yang_disebut() {
        let d = file_dialog()
            .title("Buka")
            .directory("/tmp")
            .file_name("catatan.md");
        assert_eq!(d.title.as_deref(), Some("Buka"));
        assert_eq!(d.directory.as_deref(), Some(Path::new("/tmp")));
        assert_eq!(d.file_name.as_deref(), Some("catatan.md"));
    }

    #[test]
    fn pesan_bawaan_informasi_dengan_satu_tombol() {
        let m = message("Judul");
        assert_eq!(m.level, MessageLevel::Info);
        assert_eq!(m.button_set(), MessageButtons::Ok);
        assert!(m.body.is_empty());
    }

    #[test]
    fn pesan_bisa_dinaikkan_tingkat_keseriusannya() {
        let m = message("Gagal")
            .body("Berkas tidak terbaca.")
            .level(MessageLevel::Error)
            .buttons(MessageButtons::YesNoCancel);
        assert_eq!(m.level, MessageLevel::Error);
        assert_eq!(m.button_set(), MessageButtons::YesNoCancel);
        assert_eq!(m.body, "Berkas tidak terbaca.");
    }

    #[test]
    fn jawaban_rfd_dipetakan_satu_lawan_satu() {
        assert_eq!(
            jawaban_dari_rfd(rfd::MessageDialogResult::Yes),
            MessageAnswer::Yes
        );
        assert_eq!(
            jawaban_dari_rfd(rfd::MessageDialogResult::No),
            MessageAnswer::No
        );
        assert_eq!(
            jawaban_dari_rfd(rfd::MessageDialogResult::Ok),
            MessageAnswer::Ok
        );
        assert_eq!(
            jawaban_dari_rfd(rfd::MessageDialogResult::Cancel),
            MessageAnswer::Cancel
        );
    }

    #[test]
    fn tombol_kustom_dibaca_sebagai_batal() {
        // "Something we do not understand" must never mean "go ahead".
        assert_eq!(
            jawaban_dari_rfd(rfd::MessageDialogResult::Custom("Hapus".into())),
            MessageAnswer::Cancel
        );
    }

    #[test]
    fn tingkat_dan_tombol_diterjemahkan_ke_rfd() {
        assert!(matches!(
            rfd_level(MessageLevel::Warning),
            rfd::MessageLevel::Warning
        ));
        assert!(matches!(
            rfd_buttons(MessageButtons::OkCancel),
            rfd::MessageButtons::OkCancel
        ));
    }
}
