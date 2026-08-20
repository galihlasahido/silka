//! The path, as a row of places you can go back to.
//!
//! A pure function of a `Path`, which is the only reason it can be asserted at
//! all: the widget ([`silka_widgets::breadcrumb()`]) already knows how to draw a
//! row of crumbs, elide the middle of it and keep the keyboard working. What it
//! cannot know is where each crumb *leads*, and getting that wrong is the one
//! bug a breadcrumb can have that matters — clicking "Users" and landing in
//! "Users/ana/Pictures".

use std::path::{Component, Path, PathBuf};

/// One step of the path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// What the crumb says.
    pub label: String,
    /// Where clicking it goes — always a prefix of the original path.
    pub path: PathBuf,
}

/// Split a path into crumbs, root first.
///
/// Every segment's `path` is the accumulated prefix, so segment *n* is the
/// parent of segment *n+1* and clicking anywhere in the row lands exactly where
/// it says.
///
/// ```text
/// segments("/Users/ana/Pictures")
///   -> ["/", "Users", "ana", "Pictures"]
/// ```
///
/// A relative path gets no root crumb, and `.`/`..` are kept as they are
/// written rather than resolved: this row describes the path the application is
/// showing, and quietly rewriting it would make the crumbs disagree with the
/// window.
pub fn segments(path: &Path) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut acc = PathBuf::new();
    for component in path.components() {
        acc.push(component.as_os_str());
        let label = match component {
            // The filesystem root prints as itself rather than as an empty
            // string, which is what `as_os_str().to_string_lossy()` would give
            // on a POSIX system.
            Component::RootDir => "/".to_string(),
            other => other.as_os_str().to_string_lossy().into_owned(),
        };
        out.push(Segment {
            label,
            path: acc.clone(),
        });
    }
    out
}

/// The parent of `path`, when it has one that is not itself.
///
/// `None` at the root, which is what disables the "up" button rather than
/// letting it walk in circles.
pub fn parent_of(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    (parent != path).then(|| parent.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(path: &str) -> Vec<String> {
        segments(Path::new(path))
            .into_iter()
            .map(|s| s.label)
            .collect()
    }

    #[test]
    fn setiap_remah_menunjuk_ke_awalannya_sendiri() {
        // The one bug a breadcrumb can have that matters.
        let segs = segments(Path::new("/Users/ana/Pictures"));
        assert_eq!(
            labels("/Users/ana/Pictures"),
            ["/", "Users", "ana", "Pictures"]
        );
        assert_eq!(segs[0].path, PathBuf::from("/"));
        assert_eq!(segs[1].path, PathBuf::from("/Users"));
        assert_eq!(segs[2].path, PathBuf::from("/Users/ana"));
        assert_eq!(segs[3].path, PathBuf::from("/Users/ana/Pictures"));
    }

    #[test]
    fn akar_punya_satu_remah_dan_bukan_nol() {
        assert_eq!(labels("/"), ["/"]);
        assert_eq!(segments(Path::new("/"))[0].path, PathBuf::from("/"));
    }

    #[test]
    fn lintasan_relatif_tidak_mengarang_akar() {
        assert_eq!(labels("a/b"), ["a", "b"]);
        assert_eq!(segments(Path::new("a/b"))[1].path, PathBuf::from("a/b"));
    }

    #[test]
    fn spasi_dan_bukan_ascii_tetap_utuh() {
        assert_eq!(labels("/tmp/my photos/é"), ["/", "tmp", "my photos", "é"]);
    }

    #[test]
    fn induk_akar_adalah_tidak_ada() {
        // Otherwise "up" walks in circles at the top of the filesystem.
        assert_eq!(parent_of(Path::new("/")), None);
        assert_eq!(
            parent_of(Path::new("/Users/ana")),
            Some(PathBuf::from("/Users"))
        );
    }
}
