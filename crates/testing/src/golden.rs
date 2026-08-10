//! Golden (snapshot) files: the picture this scene is supposed to draw.
//!
//! The workflow is the one Flutter uses to keep a million widgets honest, and it
//! has exactly three moves:
//!
//! ```text
//! cargo test                      compare against the committed golden
//! SILKA_GOLDEN=new  cargo test    write the goldens that do not exist yet
//! SILKA_GOLDEN=update cargo test  overwrite — you looked at the diff and the
//!                                 new picture is the correct one
//! ```
//!
//! On a mismatch the failure is not a bare "images differ": the actual capture
//! and a magenta diff are written next to the golden, and the panic message
//! names all three paths plus the bounding box of the change. That is the
//! difference between a snapshot suite people maintain and one they delete.
//!
//! ## Per-platform goldens
//!
//! A capture from Metal and a capture from lavapipe are not bit-identical, and
//! for text they are not even close. [`crate::Tolerance`] covers the
//! small drift; when a platform genuinely needs its own reference, drop a file
//! named `<name>.<os>.png` (for example `button.linux.png`) beside the shared
//! `<name>.png` and it wins on that OS. Nothing else changes — same test, same
//! name.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::diff::{compare, visualize, Diff, SizeMismatch, Tolerance};
use crate::image::Image;
use crate::png::{self, PngError};

/// The environment variable that selects the mode.
pub const MODE_ENV: &str = "SILKA_GOLDEN";
/// Overrides [`Tolerance::channel`] for every golden in the run — the knob CI
/// reaches for when a driver is noisier than the reference machine.
pub const CHANNEL_ENV: &str = "SILKA_GOLDEN_TOLERANCE";
/// Overrides [`Tolerance::different_ratio`] for every golden in the run.
pub const RATIO_ENV: &str = "SILKA_GOLDEN_RATIO";

/// What a golden assertion does when it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Compare, and fail when the golden is missing. The only mode CI may run.
    #[default]
    Compare,
    /// Write goldens that do not exist yet; compare the ones that do.
    New,
    /// Overwrite every golden with what was just captured.
    Update,
}

impl Mode {
    /// Read the mode from [`MODE_ENV`]; anything unrecognised is
    /// [`Mode::Compare`], because the safe default is the strict one.
    pub fn from_env() -> Self {
        match std::env::var(MODE_ENV).unwrap_or_default().as_str() {
            "update" | "overwrite" => Mode::Update,
            "new" | "missing" => Mode::New,
            _ => Mode::Compare,
        }
    }
}

/// What happened when a golden was asserted.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The capture matched the stored golden.
    Matched(Diff),
    /// The golden was written to disk (it was missing, or the mode said so).
    Written(PathBuf),
}

/// Why a golden assertion failed.
#[derive(Debug, Clone, PartialEq)]
pub enum GoldenFailure {
    /// No golden file exists and the mode does not allow creating one.
    Missing {
        /// Where the file was expected.
        path: PathBuf,
    },
    /// The capture is a different size than the golden.
    Size {
        /// The size mismatch itself.
        mismatch: SizeMismatch,
        /// Where the capture was written for inspection.
        actual: PathBuf,
    },
    /// The capture differs beyond the tolerance.
    Mismatch {
        /// The full comparison.
        diff: Box<Diff>,
        /// The golden that was compared against.
        golden: PathBuf,
        /// Where the capture was written.
        actual: PathBuf,
        /// Where the magenta diff was written.
        visual: PathBuf,
    },
    /// The golden file could not be read or written.
    Io {
        /// The file involved.
        path: PathBuf,
        /// The operating system's complaint.
        message: String,
    },
    /// The golden file is not a PNG this crate can read.
    Png {
        /// The file involved.
        path: PathBuf,
        /// The decoding error.
        error: PngError,
    },
}

impl fmt::Display for GoldenFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GoldenFailure::Missing { path } => write!(
                f,
                "berkas golden belum ada: {}\n  buat dengan: {MODE_ENV}=new cargo test",
                path.display()
            ),
            GoldenFailure::Size { mismatch, actual } => {
                write!(f, "{mismatch}\n  hasil tangkapan: {}", actual.display())
            }
            GoldenFailure::Mismatch {
                diff,
                golden,
                actual,
                visual,
            } => write!(
                f,
                "tangkapan tidak sama dengan golden.\n  {}\n\
                 \n  golden : {}\n  hasil  : {}\n  selisih: {}\n\
                 \n  Bila gambar baru yang benar: {MODE_ENV}=update cargo test",
                diff.report(),
                golden.display(),
                actual.display(),
                visual.display()
            ),
            GoldenFailure::Io { path, message } => {
                write!(f, "gagal mengakses {}: {message}", path.display())
            }
            GoldenFailure::Png { path, error } => {
                write!(f, "gagal membaca {}: {error}", path.display())
            }
        }
    }
}

impl std::error::Error for GoldenFailure {}

/// One golden file and the rules for comparing against it.
#[derive(Debug, Clone)]
pub struct Golden {
    name: String,
    dir: PathBuf,
    tolerance: Tolerance,
    mode: Mode,
}

impl Golden {
    /// A golden named `name`, stored in the **calling crate's**
    /// `tests/golden/` directory.
    ///
    /// The directory comes from `CARGO_MANIFEST_DIR`, which cargo sets while
    /// running tests, so each crate keeps its own goldens next to the tests
    /// that produce them rather than in one shared pile.
    pub fn new(name: impl Into<String>) -> Self {
        Self::in_dir(default_dir(), name)
    }

    /// A golden in an explicit directory.
    pub fn in_dir(dir: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            dir: dir.into(),
            tolerance: Tolerance::default(),
            mode: Mode::from_env(),
        }
    }

    /// Choose the tolerance — pick it by what the scene draws
    /// ([`Tolerance::GEOMETRY`], [`Tolerance::SHAPES`], [`Tolerance::TEXT`]).
    pub fn tolerance(mut self, tolerance: Tolerance) -> Self {
        self.tolerance = tolerance;
        self
    }

    /// Force the mode instead of reading the environment. Used by this crate's
    /// own tests, which must not depend on process-wide state.
    pub fn mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }

    /// The tolerance actually in force, after the environment overrides.
    pub fn effective_tolerance(&self) -> Tolerance {
        let mut t = self.tolerance;
        if let Some(c) = std::env::var(CHANNEL_ENV).ok().and_then(|v| v.parse().ok()) {
            t.channel = c;
        }
        if let Some(r) = std::env::var(RATIO_ENV).ok().and_then(|v| v.parse().ok()) {
            t.different_ratio = r;
        }
        t
    }

    /// The golden's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The file this golden will be compared against: the platform-specific
    /// override when one exists, the shared file otherwise.
    pub fn path(&self) -> PathBuf {
        let specific = self.dir.join(format!("{}.{}.png", self.name, os_slug()));
        if specific.exists() {
            specific
        } else {
            self.dir.join(format!("{}.png", self.name))
        }
    }

    /// Where a failing capture is written.
    pub fn actual_path(&self) -> PathBuf {
        self.dir.join("actual").join(format!("{}.png", self.name))
    }

    /// Where the magenta diff is written.
    pub fn visual_path(&self) -> PathBuf {
        self.dir
            .join("actual")
            .join(format!("{}.diff.png", self.name))
    }

    /// Compare without panicking — the form this crate's own tests use.
    pub fn check(&self, capture: &Image) -> Result<Outcome, GoldenFailure> {
        let path = self.path();
        if self.mode == Mode::Update || (self.mode == Mode::New && !path.exists()) {
            return self.write(&path, capture).map(Outcome::Written);
        }
        if !path.exists() {
            return Err(GoldenFailure::Missing { path });
        }

        let bytes = std::fs::read(&path).map_err(|e| GoldenFailure::Io {
            path: path.clone(),
            message: e.to_string(),
        })?;
        let golden = png::decode(&bytes).map_err(|error| GoldenFailure::Png {
            path: path.clone(),
            error,
        })?;

        let tolerance = self.effective_tolerance();
        match compare(&golden, capture, tolerance) {
            Err(mismatch) => {
                let actual = self.write(&self.actual_path(), capture)?;
                Err(GoldenFailure::Size { mismatch, actual })
            }
            Ok(diff) if diff.is_match() => Ok(Outcome::Matched(diff)),
            Ok(diff) => {
                let actual = self.write(&self.actual_path(), capture)?;
                let visual =
                    self.write(&self.visual_path(), &visualize(&golden, capture, tolerance))?;
                Err(GoldenFailure::Mismatch {
                    diff: Box::new(diff),
                    golden: path,
                    actual,
                    visual,
                })
            }
        }
    }

    /// Compare, and panic with a report a human can act on.
    ///
    /// Returns the diff on success so a test can additionally assert something
    /// about *how* close it was.
    pub fn assert(&self, capture: &Image) -> Diff {
        match self.check(capture) {
            Ok(Outcome::Matched(diff)) => diff,
            Ok(Outcome::Written(path)) => {
                eprintln!("golden ditulis: {}", path.display());
                Diff {
                    total: capture.pixel_count(),
                    different: 0,
                    max_channel: 0,
                    worst_at: None,
                    bounds: None,
                    tolerance: self.effective_tolerance(),
                }
            }
            Err(failure) => panic!("golden {:?}: {failure}", self.name),
        }
    }

    fn write(&self, path: &Path, image: &Image) -> Result<PathBuf, GoldenFailure> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| GoldenFailure::Io {
                path: parent.to_path_buf(),
                message: e.to_string(),
            })?;
        }
        std::fs::write(path, png::encode(image)).map_err(|e| GoldenFailure::Io {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
        Ok(path.to_path_buf())
    }
}

/// `<crate>/tests/golden`, or the current directory when cargo is not the one
/// running us.
fn default_dir() -> PathBuf {
    match std::env::var("CARGO_MANIFEST_DIR") {
        Ok(dir) => PathBuf::from(dir).join("tests").join("golden"),
        Err(_) => PathBuf::from("tests").join("golden"),
    }
}

/// The suffix a per-platform golden uses.
fn os_slug() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A private directory per test — the suite runs in parallel and goldens
    /// are files.
    fn temp_dir(label: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("silka-golden-{}-{label}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("buat direktori sementara");
        dir
    }

    fn gambar(warna: [u8; 4]) -> Image {
        Image::filled(16, 12, warna)
    }

    #[test]
    fn mode_compare_gagal_bila_golden_belum_ada() {
        let dir = temp_dir("hilang");
        let g = Golden::in_dir(&dir, "belum-ada").mode(Mode::Compare);
        let e = g.check(&gambar([1, 2, 3, 255])).unwrap_err();
        assert!(matches!(e, GoldenFailure::Missing { .. }));
        assert!(e.to_string().contains("SILKA_GOLDEN=new"), "{e}");
    }

    #[test]
    fn mode_new_menulis_lalu_membandingkan() {
        let dir = temp_dir("baru");
        let g = Golden::in_dir(&dir, "kotak").mode(Mode::New);
        let img = gambar([9, 9, 9, 255]);
        assert!(matches!(g.check(&img), Ok(Outcome::Written(_))));
        assert!(dir.join("kotak.png").exists());
        // Second run compares instead of rewriting.
        assert!(matches!(g.check(&img), Ok(Outcome::Matched(_))));
    }

    #[test]
    fn mode_update_menimpa_yang_sudah_ada() {
        let dir = temp_dir("timpa");
        let g = Golden::in_dir(&dir, "kotak").mode(Mode::New);
        g.check(&gambar([0, 0, 0, 255])).expect("tulis awal");

        let baru = gambar([255, 0, 0, 255]);
        let g = g.mode(Mode::Update);
        assert!(matches!(g.check(&baru), Ok(Outcome::Written(_))));

        let g = g.mode(Mode::Compare);
        assert!(matches!(g.check(&baru), Ok(Outcome::Matched(_))));
    }

    #[test]
    fn ketidakcocokan_menulis_hasil_dan_selisih() {
        let dir = temp_dir("beda");
        let g = Golden::in_dir(&dir, "kotak")
            .mode(Mode::New)
            .tolerance(Tolerance::EXACT);
        g.check(&gambar([0, 0, 0, 255])).expect("tulis awal");

        let g = g.mode(Mode::Compare);
        let e = g.check(&gambar([255, 255, 255, 255])).unwrap_err();
        let GoldenFailure::Mismatch {
            diff,
            actual,
            visual,
            ..
        } = &e
        else {
            panic!("harus Mismatch, bukan {e:?}");
        };
        assert_eq!(diff.different, 16 * 12);
        assert!(actual.exists(), "hasil tangkapan harus ditulis");
        assert!(visual.exists(), "gambar selisih harus ditulis");
        // Both artefacts must be readable back — a diff nobody can open is a
        // diff nobody looks at.
        let v = png::decode(&std::fs::read(visual).expect("baca selisih")).expect("dekode");
        assert_eq!((v.width(), v.height()), (16, 12));
        assert!(e.to_string().contains("SILKA_GOLDEN=update"), "{e}");
    }

    #[test]
    fn ukuran_berbeda_dilaporkan_terpisah() {
        let dir = temp_dir("ukuran");
        let g = Golden::in_dir(&dir, "kotak").mode(Mode::New);
        g.check(&Image::filled(8, 8, [0, 0, 0, 255]))
            .expect("tulis");
        let g = g.mode(Mode::Compare);
        let e = g.check(&Image::filled(8, 9, [0, 0, 0, 255])).unwrap_err();
        assert!(matches!(e, GoldenFailure::Size { .. }), "{e:?}");
    }

    #[test]
    fn golden_khusus_platform_menang_bila_ada() {
        let dir = temp_dir("platform");
        let g = Golden::in_dir(&dir, "kotak");
        assert_eq!(g.path(), dir.join("kotak.png"), "tanpa berkas khusus");

        std::fs::write(dir.join(format!("kotak.{}.png", os_slug())), b"x").expect("tulis");
        assert_eq!(
            g.path(),
            dir.join(format!("kotak.{}.png", os_slug())),
            "berkas khusus platform harus menang"
        );
    }

    #[test]
    fn berkas_golden_rusak_dilaporkan_sebagai_png_bukan_ketidakcocokan() {
        let dir = temp_dir("rusak");
        std::fs::write(dir.join("kotak.png"), b"jelas bukan png").expect("tulis");
        let g = Golden::in_dir(&dir, "kotak").mode(Mode::Compare);
        let e = g.check(&gambar([0, 0, 0, 255])).unwrap_err();
        assert!(matches!(e, GoldenFailure::Png { .. }), "{e:?}");
    }

    #[test]
    fn mode_dari_lingkungan_default_ke_yang_ketat() {
        // Whatever the environment says, an unknown value must never turn the
        // suite into one that rewrites its own expectations.
        assert_eq!(Mode::default(), Mode::Compare);
    }
}
