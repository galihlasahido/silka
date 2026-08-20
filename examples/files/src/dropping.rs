//! Accepting a drop **from** outside the application — the other half of
//! `INTEGRASI-NATIVE.md` §4, and the easy half: winit already reports it.
//!
//! What is not easy is deciding what a drop *means*, and this module is that
//! decision written down as pure functions:
//!
//! - **A drop is a copy, never a move.** winit hands over a list of paths and
//!   nothing else — no effect, no modifier state, no source. Guessing "move"
//!   from a list of paths means deleting a file in another application's folder
//!   because this one assumed something. Copying is the only interpretation the
//!   information supports.
//! - **A name that is taken does not overwrite.** [`unique_name`] puts the
//!   suffix before the extension, so a dropped `photo.png` becomes
//!   `photo 2.png` and is still a picture to everything that reads extensions.
//! - **A folder cannot be dropped into itself.** [`is_ancestor`] is the guard,
//!   and it is not hypothetical: a recursive copy into its own destination
//!   fills a disk.

use std::path::{Path, PathBuf};

/// One file's journey, decided before anything touches the disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copy {
    /// Where it comes from.
    pub from: PathBuf,
    /// Where it will land — always inside the target folder.
    pub to: PathBuf,
}

/// Why a dropped path is not going to be copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// It is already in the target folder.
    AlreadyHere,
    /// It is the target folder, or contains it.
    WouldRecurse,
    /// It has no last path component to name the copy after.
    Nameless,
}

/// What a drop will do, in full, before it does any of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropPlan {
    /// The folder receiving the drop.
    pub target: PathBuf,
    /// The copies to perform, in order.
    pub copies: Vec<Copy>,
    /// The paths that will be left alone, and why.
    pub skipped: Vec<(PathBuf, Skip)>,
}

impl DropPlan {
    /// Whether the drop will do anything at all.
    pub fn is_empty(&self) -> bool {
        self.copies.is_empty()
    }

    /// A one-line summary for the status bar.
    pub fn describe(&self) -> String {
        match (self.copies.len(), self.skipped.len()) {
            (0, 0) => "Nothing dropped".to_string(),
            (0, n) => format!("{n} skipped"),
            (1, 0) => "Copying 1 item".to_string(),
            (n, 0) => format!("Copying {n} items"),
            (n, s) => format!("Copying {n} items, {s} skipped"),
        }
    }
}

/// Decide what a drop of `paths` into `target` will do.
///
/// `taken` answers "does this name already exist in the target folder?".
/// Passing it in is what makes the whole decision testable without a
/// filesystem — and it has to account for names this very plan has already
/// claimed, which is why the plan feeds its own choices back in.
pub fn plan(paths: &[PathBuf], target: &Path, taken: impl Fn(&str) -> bool) -> DropPlan {
    let mut copies = Vec::new();
    let mut skipped = Vec::new();
    let mut claimed: Vec<String> = Vec::new();

    for path in paths {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            skipped.push((path.clone(), Skip::Nameless));
            continue;
        };
        if path.parent() == Some(target) {
            skipped.push((path.clone(), Skip::AlreadyHere));
            continue;
        }
        if is_ancestor(path, target) {
            skipped.push((path.clone(), Skip::WouldRecurse));
            continue;
        }
        // Both the filesystem and the names this plan has already handed out:
        // dropping `a/photo.png` and `b/photo.png` together must produce two
        // files, not one file written twice.
        let unique = unique_name(name, |candidate| {
            taken(candidate) || claimed.iter().any(|c| c == candidate)
        });
        claimed.push(unique.clone());
        copies.push(Copy {
            from: path.clone(),
            to: target.join(unique),
        });
    }

    DropPlan {
        target: target.to_path_buf(),
        copies,
        skipped,
    }
}

/// Whether `ancestor` is `descendant` or contains it.
///
/// The guard that stops a folder being copied into its own subfolder, which is
/// a recursive copy that ends when the disk is full.
pub fn is_ancestor(ancestor: &Path, descendant: &Path) -> bool {
    descendant == ancestor || descendant.starts_with(ancestor)
}

/// A name that is free in the destination.
///
/// The suffix goes **before** the extension — `photo 2.png`, not `photo.png 2`
/// — so the copy is still a picture to every tool that looks at extensions.
/// A leading dot is a whole name and not an extension: `.bashrc` becomes
/// `.bashrc 2`.
///
/// ```text
/// unique_name("photo.png", |_| false)              == "photo.png"
/// unique_name("photo.png", |n| n == "photo.png")   == "photo 2.png"
/// ```
pub fn unique_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_string();
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, Some(ext)),
        _ => (name, None),
    };
    for n in 2..10_000u32 {
        let candidate = match extension {
            Some(ext) => format!("{stem} {n}.{ext}"),
            None => format!("{stem} {n}"),
        };
        if !taken(&candidate) {
            return candidate;
        }
    }
    // Ten thousand copies of one name in one folder is not a case worth its own
    // error; the caller will get a plain "already exists" from the filesystem.
    name.to_string()
}

/// Copy one file or directory tree. **Blocking** — this is task work.
///
/// Recursive by hand rather than through a crate, because the recursion is
/// three lines and the dependency would be the fourth. Symlinks are followed
/// like any other entry: a file explorer that silently turned an alias into a
/// copy of its target would be lying, but so would one that refused, and
/// following is what the platform's own copy does.
pub fn copy_tree(from: &Path, to: &Path) -> std::io::Result<u64> {
    let meta = std::fs::metadata(from)?;
    if !meta.is_dir() {
        return std::fs::copy(from, to);
    }
    std::fs::create_dir_all(to)?;
    let mut total = 0;
    for item in std::fs::read_dir(from)? {
        let item = item?;
        total += copy_tree(&item.path(), &to.join(item.file_name()))?;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn nama_bentrok_tidak_menimpa_apa_pun() {
        assert_eq!(unique_name("photo.png", |_| false), "photo.png");
        assert_eq!(
            unique_name("photo.png", |n| n == "photo.png"),
            "photo 2.png"
        );
        assert_eq!(
            unique_name("photo.png", |n| n == "photo.png" || n == "photo 2.png"),
            "photo 3.png"
        );
    }

    #[test]
    fn akhiran_ditaruh_sebelum_ekstensi() {
        // So the copy is still a PNG to everything that reads extensions.
        assert!(unique_name("photo.png", |n| n == "photo.png").ends_with(".png"));
        // A leading dot is a name, not an extension.
        assert_eq!(unique_name(".bashrc", |n| n == ".bashrc"), ".bashrc 2");
        // And a name with no extension keeps its shape.
        assert_eq!(unique_name("README", |n| n == "README"), "README 2");
    }

    #[test]
    fn rencana_menyalin_ke_dalam_folder_tujuan() {
        let plan = plan(
            &[p("/other/a.txt"), p("/other/b.txt")],
            Path::new("/here"),
            |_| false,
        );
        assert_eq!(plan.copies.len(), 2);
        assert_eq!(plan.copies[0].to, p("/here/a.txt"));
        assert_eq!(plan.copies[1].to, p("/here/b.txt"));
        assert!(plan.skipped.is_empty());
        assert_eq!(plan.describe(), "Copying 2 items");
    }

    #[test]
    fn dua_berkas_bernama_sama_menjadi_dua_berkas() {
        // The bug this guards: both copies write to the same destination and
        // the second silently destroys the first.
        let plan = plan(
            &[p("/a/photo.png"), p("/b/photo.png")],
            Path::new("/here"),
            |_| false,
        );
        assert_eq!(plan.copies.len(), 2);
        assert_eq!(plan.copies[0].to, p("/here/photo.png"));
        assert_eq!(plan.copies[1].to, p("/here/photo 2.png"));
    }

    #[test]
    fn menjatuhkan_ke_folder_yang_sama_tidak_melakukan_apa_apa() {
        let plan = plan(&[p("/here/a.txt")], Path::new("/here"), |_| false);
        assert!(plan.is_empty());
        assert_eq!(plan.skipped, vec![(p("/here/a.txt"), Skip::AlreadyHere)]);
        assert_eq!(plan.describe(), "1 skipped");
    }

    #[test]
    fn folder_tidak_bisa_dijatuhkan_ke_dalam_dirinya_sendiri() {
        // A recursive copy that ends when the disk is full.
        assert!(is_ancestor(Path::new("/a"), Path::new("/a/b/c")));
        assert!(is_ancestor(Path::new("/a"), Path::new("/a")));
        assert!(!is_ancestor(Path::new("/a"), Path::new("/b")));

        let plan = plan(&[p("/a")], Path::new("/a/b"), |_| false);
        assert!(plan.is_empty());
        assert_eq!(plan.skipped, vec![(p("/a"), Skip::WouldRecurse)]);
    }

    #[test]
    fn nama_yang_sudah_dipakai_di_tujuan_dihindari() {
        let plan = plan(&[p("/other/a.txt")], Path::new("/here"), |n| n == "a.txt");
        assert_eq!(plan.copies[0].to, p("/here/a 2.txt"));
    }

    #[test]
    fn lintasan_tanpa_nama_dilewati_bukan_membuat_rencana_omong_kosong() {
        let plan = plan(&[p("/")], Path::new("/here"), |_| false);
        assert_eq!(plan.skipped, vec![(p("/"), Skip::Nameless)]);
    }

    #[test]
    fn ringkasan_menyebut_keduanya() {
        let plan = plan(
            &[p("/other/a.txt"), p("/here/b.txt")],
            Path::new("/here"),
            |_| false,
        );
        assert_eq!(plan.describe(), "Copying 1 items, 1 skipped");
        assert_eq!(
            DropPlan {
                target: p("/here"),
                copies: Vec::new(),
                skipped: Vec::new()
            }
            .describe(),
            "Nothing dropped"
        );
    }

    #[test]
    fn menyalin_pohon_menyalin_seluruh_isinya() {
        let root = std::env::temp_dir().join("silka-files-copy-tree");
        let _ = std::fs::remove_dir_all(&root);
        let from = root.join("from");
        std::fs::create_dir_all(from.join("deep")).expect("temp dirs");
        std::fs::write(from.join("a.txt"), b"hello").expect("write");
        std::fs::write(from.join("deep/b.txt"), b"there").expect("write");

        let to = root.join("to");
        copy_tree(&from, &to).expect("copy");
        assert_eq!(std::fs::read(to.join("a.txt")).expect("read"), b"hello");
        assert_eq!(
            std::fs::read(to.join("deep/b.txt")).expect("read"),
            b"there"
        );
        // …and the original is untouched, which is what "copy" means.
        assert!(from.join("a.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
