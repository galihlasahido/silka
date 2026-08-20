//! What one line of a folder listing *is*, before anything draws it.
//!
//! Everything in this module is a pure function of a directory entry, which is
//! why it is all testable without a disk: the kind a name implies, the icon and
//! colour that kind gets, how a byte count is written for a human, and the
//! order the rows come in.
//!
//! The one decision worth defending here is the **sort**. Folders come before
//! files, and within each group names are compared with the digit runs read as
//! numbers, so `file2` sorts before `file10`. Every file manager a user has
//! ever used does this, and the plain byte-order alternative puts `file10`
//! second — a difference nobody can articulate but everybody notices.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use silka_paint::Color;
use silka_theme::{ColorToken, Theme};
use silka_widgets::IconName;

/// What a file is, as far as a listing is concerned.
///
/// Deliberately coarse. A file explorer that distinguishes forty types has
/// forty icons nobody recognises; these eight are the ones that change what a
/// row *means* — you can open it, you can look inside it, or you cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileKind {
    /// A directory.
    Folder,
    /// A picture — the one kind that gets a thumbnail instead of an icon.
    Image,
    /// Plain text, Markdown, a log.
    Text,
    /// Source code or a configuration file.
    Code,
    /// A document with a format of its own: PDF, an office file.
    Document,
    /// An archive.
    Archive,
    /// Audio or video.
    Media,
    /// Anything else.
    Other,
}

impl FileKind {
    /// Every kind, in one place — so a kind added later cannot quietly miss an
    /// icon, a tint or a name.
    #[cfg(test)]
    pub const ALL: [FileKind; 8] = [
        FileKind::Folder,
        FileKind::Image,
        FileKind::Text,
        FileKind::Code,
        FileKind::Document,
        FileKind::Archive,
        FileKind::Media,
        FileKind::Other,
    ];

    /// The kind a path implies.
    ///
    /// Extension only — no sniffing of file contents. Reading the first bytes
    /// of every file in a folder of ten thousand is exactly the kind of work
    /// this example exists to prove it does not do.
    pub fn of(path: &Path, is_dir: bool) -> FileKind {
        if is_dir {
            return FileKind::Folder;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match ext.as_str() {
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "heic" | "tiff" | "svg" => {
                FileKind::Image
            }
            "txt" | "md" | "markdown" | "log" | "csv" | "rtf" => FileKind::Text,
            "rs" | "toml" | "json" | "yaml" | "yml" | "js" | "ts" | "py" | "c" | "h" | "cpp"
            | "go" | "java" | "swift" | "sh" | "html" | "css" | "wgsl" => FileKind::Code,
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "pages" | "numbers"
            | "key" | "epub" => FileKind::Document,
            "zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "dmg" => FileKind::Archive,
            "mp3" | "wav" | "flac" | "aac" | "m4a" | "mp4" | "mov" | "mkv" | "avi" | "webm" => {
                FileKind::Media
            }
            _ => FileKind::Other,
        }
    }

    /// Whether a thumbnail is worth attempting.
    ///
    /// Narrower than [`FileKind::Image`] on purpose: the kind covers every
    /// picture format a user recognises, but only two of them have a decoder
    /// compiled into this example, and offering to preview a HEIC that will
    /// always fail is worse than not offering.
    pub fn is_previewable(path: &Path) -> bool {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        matches!(ext.as_str(), "png" | "jpg" | "jpeg")
    }

    /// The symbol shown when there is no thumbnail.
    pub fn icon(self) -> IconName {
        match self {
            // A folder has no glyph of its own in the built-in set; the
            // chevron is what the tree uses for the same thing, so the listing
            // agrees with the sidebar rather than inventing a second language.
            FileKind::Folder => IconName::ChevronRight,
            FileKind::Image => IconName::Star,
            FileKind::Text => IconName::Menu,
            FileKind::Code => IconName::Ellipsis,
            FileKind::Document => IconName::Calendar,
            FileKind::Archive => IconName::Download,
            FileKind::Media => IconName::Bell,
            FileKind::Other => IconName::Info,
        }
    }

    /// The colour the icon is tinted with — a **token**, resolved against the
    /// live theme, never a literal (§2.6).
    pub fn tint(self, theme: &Theme) -> Color {
        let token = match self {
            FileKind::Folder => ColorToken::Accent,
            FileKind::Image => ColorToken::Success,
            FileKind::Text => ColorToken::Label,
            FileKind::Code => ColorToken::AccentHover,
            FileKind::Document => ColorToken::Warning,
            FileKind::Archive => ColorToken::SecondaryLabel,
            FileKind::Media => ColorToken::Destructive,
            FileKind::Other => ColorToken::TertiaryLabel,
        };
        theme.resolve(token)
    }

    /// The word a screen reader hears in place of the icon.
    pub fn describe(self) -> &'static str {
        match self {
            FileKind::Folder => "Folder",
            FileKind::Image => "Image",
            FileKind::Text => "Text",
            FileKind::Code => "Code",
            FileKind::Document => "Document",
            FileKind::Archive => "Archive",
            FileKind::Media => "Media",
            FileKind::Other => "File",
        }
    }
}

/// One row of a folder listing.
///
/// Everything here is read **once**, while the directory is being scanned off
/// the UI thread. A row that had to ask the filesystem for its own size while
/// it was being drawn would put a `stat` call inside a scroll frame, which is
/// the classic way a file list becomes unusable on a network volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The full path.
    pub path: PathBuf,
    /// The last component, as shown.
    pub name: String,
    /// Whether it is a directory (after following symlinks).
    pub is_dir: bool,
    /// Size in bytes; meaningless for a directory and reported as zero.
    pub size: u64,
    /// Last modification time, when the filesystem would say.
    pub modified: Option<SystemTime>,
    /// The kind, computed once here rather than per frame.
    pub kind: FileKind,
}

impl Entry {
    /// Build a row from the parts a directory scan already has in hand.
    pub fn new(path: PathBuf, is_dir: bool, size: u64, modified: Option<SystemTime>) -> Self {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
            // A path that ends in `..` or is a bare root has no last
            // component; showing the whole path is better than showing an
            // empty row the user cannot click.
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let kind = FileKind::of(&path, is_dir);
        Self {
            path,
            name,
            is_dir,
            size: if is_dir { 0 } else { size },
            modified,
            kind,
        }
    }

    /// Whether the name starts with a dot — the Unix convention for "hidden".
    pub fn is_hidden(&self) -> bool {
        self.name.starts_with('.')
    }

    /// The size column's text; a folder gets an em dash rather than `0 B`,
    /// because zero is a claim about a folder that is almost never true.
    pub fn size_text(&self) -> String {
        if self.is_dir {
            "—".to_string()
        } else {
            bytes(self.size)
        }
    }
}

/// Bytes as a human reads them: `1.50 kB`, `13.4 GB`.
///
/// Decimal units, matching what the platform's own file managers show. A file
/// explorer that disagrees with Finder about the size of a file is a file
/// explorer nobody trusts, whichever one is technically right.
///
/// ```text
/// bytes(0)         == "0 B"
/// bytes(999)       == "999 B"
/// bytes(1_500)     == "1.50 kB"
/// bytes(13_400_000) == "13.4 MB"
/// ```
pub fn bytes(value: u64) -> String {
    const UNITS: [&str; 6] = ["B", "kB", "MB", "GB", "TB", "PB"];
    let mut v = value as f64;
    let mut unit = 0;
    while v >= 1000.0 && unit + 1 < UNITS.len() {
        v /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value} B")
    } else if v < 10.0 {
        format!("{v:.2} {}", UNITS[unit])
    } else if v < 100.0 {
        format!("{v:.1} {}", UNITS[unit])
    } else {
        format!("{v:.0} {}", UNITS[unit])
    }
}

/// A modification time as `YYYY-MM-DD HH:MM`.
///
/// UTC, and for the same reason `silka_platform::trash::deletion_date` gives
/// UTC: turning an instant into a local wall clock needs a timezone database
/// this example is not going to carry, and a date that is silently an hour
/// wrong twice a year is worse than one that is honestly universal.
pub fn timestamp(time: Option<SystemTime>) -> String {
    let Some(time) = time else {
        return "—".to_string();
    };
    let secs = match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        // A file dated before 1970 is a corrupt timestamp far more often than
        // it is a real one; either way there is nothing useful to print.
        Err(_) => return "—".to_string(),
    };
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute) = (rest / 3600, (rest % 3600) / 60);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

/// Days since the Unix epoch as a civil (proleptic Gregorian) date.
///
/// Howard Hinnant's `civil_from_days`, the same one `silka_platform::trash`
/// uses — exact for every date that matters and with no leap-year branch to
/// get wrong.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (y + i64::from(m <= 2), m as u32, d as u32)
}

// ---------------------------------------------------------------------------
// Order
// ---------------------------------------------------------------------------

/// The listing order: folders first, then names compared naturally.
pub fn compare(a: &Entry, b: &Entry) -> Ordering {
    b.is_dir
        .cmp(&a.is_dir)
        .then_with(|| natural(&a.name, &b.name))
        // Two entries with the same name in one directory is impossible, but a
        // merged listing (a drop preview, say) can hold both — and a sort that
        // is not a total order makes `sort_unstable_by` produce garbage.
        .then_with(|| a.path.cmp(&b.path))
}

/// Compare two names with runs of digits read as numbers.
///
/// `file2` before `file10`, which is what every file manager does and what a
/// plain byte comparison gets backwards. Case is folded for the comparison but
/// not for the tie-break, so `A` and `a` still have a stable order.
pub fn natural(a: &str, b: &str) -> Ordering {
    let mut ai = a.char_indices().peekable();
    let mut bi = b.char_indices().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some((ap, ac)), Some((bp, bc))) => {
                if ac.is_ascii_digit() && bc.is_ascii_digit() {
                    let (an, anext) = digits(a, ap);
                    let (bn, bnext) = digits(b, bp);
                    if an != bn {
                        return an.cmp(&bn);
                    }
                    while ai.peek().is_some_and(|(p, _)| *p < anext) {
                        ai.next();
                    }
                    while bi.peek().is_some_and(|(p, _)| *p < bnext) {
                        bi.next();
                    }
                } else {
                    let al = ac.to_ascii_lowercase();
                    let bl = bc.to_ascii_lowercase();
                    if al != bl {
                        return al.cmp(&bl);
                    }
                    ai.next();
                    bi.next();
                }
            }
        }
    }
}

/// The digit run starting at `from`, as a number, plus the byte index after it.
///
/// Saturating rather than wrapping: a name made of two hundred digits is not a
/// version number, and a wrapped `u64` would sort it somewhere absurd.
fn digits(s: &str, from: usize) -> (u64, usize) {
    let mut value: u64 = 0;
    let mut end = from;
    for (i, c) in s[from..].char_indices() {
        if !c.is_ascii_digit() {
            break;
        }
        value = value
            .saturating_mul(10)
            .saturating_add(u64::from(c as u8 - b'0'));
        end = from + i + c.len_utf8();
    }
    (value, end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn entry(name: &str, is_dir: bool) -> Entry {
        Entry::new(PathBuf::from("/tmp").join(name), is_dir, 0, None)
    }

    #[test]
    fn jenis_dibaca_dari_ekstensi_tanpa_menyentuh_isi() {
        assert_eq!(
            FileKind::of(Path::new("/a/photo.JPG"), false),
            FileKind::Image
        );
        assert_eq!(
            FileKind::of(Path::new("/a/notes.md"), false),
            FileKind::Text
        );
        assert_eq!(FileKind::of(Path::new("/a/main.rs"), false), FileKind::Code);
        assert_eq!(
            FileKind::of(Path::new("/a/report.pdf"), false),
            FileKind::Document
        );
        assert_eq!(
            FileKind::of(Path::new("/a/bundle.tar"), false),
            FileKind::Archive
        );
        assert_eq!(
            FileKind::of(Path::new("/a/song.mp3"), false),
            FileKind::Media
        );
        assert_eq!(FileKind::of(Path::new("/a/thing"), false), FileKind::Other);
        // A directory is a folder whatever it happens to be called.
        assert_eq!(
            FileKind::of(Path::new("/a/photo.jpg"), true),
            FileKind::Folder
        );
    }

    #[test]
    fn pratinjau_hanya_ditawarkan_untuk_format_yang_bisa_dibaca() {
        // The kind is broad; the offer is narrow. A preview that always fails
        // is a worse promise than no preview at all.
        assert_eq!(FileKind::of(Path::new("/a/x.heic"), false), FileKind::Image);
        assert!(!FileKind::is_previewable(Path::new("/a/x.heic")));
        assert!(FileKind::is_previewable(Path::new("/a/x.PNG")));
        assert!(FileKind::is_previewable(Path::new("/a/x.jpeg")));
        assert!(!FileKind::is_previewable(Path::new("/a/x.txt")));
    }

    #[test]
    fn setiap_jenis_punya_ikon_dan_nama_bacaan() {
        for kind in FileKind::ALL {
            assert!(!kind.describe().is_empty(), "{kind:?}");
            assert!(!kind.icon().path().is_empty(), "{kind:?}");
        }
    }

    #[test]
    fn ukuran_ditulis_seperti_manusia_membacanya() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(999), "999 B");
        assert_eq!(bytes(1_000), "1.00 kB");
        assert_eq!(bytes(1_500), "1.50 kB");
        assert_eq!(bytes(13_400_000), "13.4 MB");
        assert_eq!(bytes(999_000_000_000_000_000), "999 PB");
    }

    #[test]
    fn folder_tidak_mengaku_berukuran_nol() {
        let dir = entry("src", true);
        assert_eq!(dir.size_text(), "—");
        let file = Entry::new(PathBuf::from("/tmp/a.txt"), false, 42, None);
        assert_eq!(file.size_text(), "42 B");
    }

    #[test]
    fn waktu_diformat_dan_yang_tidak_ada_tidak_dikarang() {
        assert_eq!(timestamp(None), "—");
        assert_eq!(timestamp(Some(SystemTime::UNIX_EPOCH)), "1970-01-01 00:00");
        // A leap day, the case a hand-rolled calendar gets wrong.
        assert_eq!(
            timestamp(Some(
                SystemTime::UNIX_EPOCH + Duration::from_secs(1_582_934_400)
            )),
            "2020-02-29 00:00"
        );
        assert_eq!(
            timestamp(Some(
                SystemTime::UNIX_EPOCH + Duration::from_secs(1_582_934_400 + 9 * 3600 + 5 * 60)
            )),
            "2020-02-29 09:05"
        );
    }

    #[test]
    fn urutan_alami_menaruh_file2_sebelum_file10() {
        // The whole reason `natural` exists: byte order says otherwise.
        assert_eq!(natural("file2", "file10"), Ordering::Less);
        assert_eq!(natural("file10", "file2"), Ordering::Greater);
        assert!("file10" < "file2", "byte order really does disagree");
        // Equal numbers keep comparing after the run.
        assert_eq!(natural("a01b", "a1c"), Ordering::Less);
        // Case is folded, so `Apple` and `apple` sit next to each other rather
        // than in two separate alphabets.
        assert_eq!(natural("Banana", "apple"), Ordering::Greater);
    }

    #[test]
    fn deretan_angka_panjang_tidak_meluap() {
        let long = "9".repeat(40);
        // Saturating, not wrapping: the comparison stays sane instead of
        // sorting a 40-digit name somewhere arbitrary.
        assert_eq!(natural(&long, "1"), Ordering::Greater);
    }

    #[test]
    fn folder_selalu_di_atas_berkas() {
        let mut rows = [
            entry("zebra", true),
            entry("apple.txt", false),
            entry("file10.txt", false),
            entry("file2.txt", false),
            entry("Beta", true),
        ];
        rows.sort_by(compare);
        let names: Vec<&str> = rows.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            ["Beta", "zebra", "apple.txt", "file2.txt", "file10.txt"]
        );
    }

    #[test]
    fn urutan_adalah_urutan_total() {
        // Two rows that agree on everything but the path still have an order,
        // which is what `sort_unstable_by` requires to be correct at all.
        let a = Entry::new(PathBuf::from("/x/a.txt"), false, 1, None);
        let b = Entry::new(PathBuf::from("/y/a.txt"), false, 1, None);
        assert_eq!(compare(&a, &a), Ordering::Equal);
        assert_ne!(compare(&a, &b), Ordering::Equal);
        assert_eq!(compare(&a, &b).reverse(), compare(&b, &a));
    }

    #[test]
    fn berkas_tersembunyi_dikenali_dari_titik_di_depan() {
        assert!(entry(".bashrc", false).is_hidden());
        assert!(!entry("bashrc", false).is_hidden());
    }
}
