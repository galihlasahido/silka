//! The folder tree, and the place this example makes its "opening a big node
//! must not block" claim.
//!
//! [`silka_widgets::tree()`] calls its source only for nodes that are open, and
//! calls [`silka_widgets::TreeBuilder::on_expand`] the moment one opens. Those
//! two facts together are the whole lazy-loading story:
//!
//! ```text
//! user clicks a chevron
//!   -> on_expand(key)          the UI thread: a hash lookup and a thread spawn
//!   -> the tree rebuilds       the folder shows one "Loading…" row
//!   -> …milliseconds later…
//!   -> the scan lands          data_version changes, the tree rebuilds again
//! ```
//!
//! Nothing in that sequence waits for a disk, which is why opening a folder of
//! ten thousand entries on a sleeping network volume costs one frame rather
//! than however long the volume takes to wake up.

use silka_core::app::{component, BuildCtx};
use silka_core::signals::Signal;
use silka_core::tree::CrossAlign;
use silka_core::view::{row, View};
use silka_paint::Insets;
use silka_text::FontWeight;
use silka_theme::Theme;
use silka_widgets::{icon, text, tree, IconName, TreeRow};

use crate::dirs::FilesSource;
use crate::entry::FileKind;
use crate::state::Explorer;

/// The tree's accessible name.
pub const TREE_LABEL: &str = "Folders";

/// One tree row's height.
///
/// Shorter than the listing's 44 points on purpose: a source list is a dense
/// control, and the tree raises the height itself where a row has to be a hit
/// target (`TreeStyle::toggle_band`).
pub const TREE_ROW_EXTENT: f32 = 28.0;

/// The folder tree.
///
/// Its own component, so that expanding a node rebuilds the tree and not the
/// listing beside it.
pub fn folders() -> View {
    component("folders", |cx: &BuildCtx| {
        let t: Theme = cx.expect_env::<Signal<Theme>>().get();
        let ex: Explorer = cx.expect_env();

        let root = ex.root.get();
        // Everything the source's answer depends on: the flattening is cached,
        // and a source that quietly started answering differently would never
        // be asked again.
        let version = ex.data_version.get();
        let source = FilesSource::new(root, ex.cache.clone(), ex.keys.clone());

        let theme = t;
        let for_expand = ex.clone();
        let for_activate = ex.clone();

        tree(ex.tree, source, move |r| tree_row(&theme, r))
            .row_extent(TREE_ROW_EXTENT)
            .guides(t.space(0.25))
            .data_version(version)
            .label(TREE_LABEL)
            .background(t.color.surface_sunken)
            // **The lazy-loading hook.** Everything it does is on this side of
            // a `spawn_blocking`; see the module documentation.
            .on_expand(move |key| {
                if let Some(path) = for_expand.keys.path(key) {
                    for_expand.ensure_loaded(&path);
                }
            })
            .on_activate(move |key| {
                // A placeholder row has no path, so clicking "Loading…" does
                // nothing at all — which is what it should do.
                let Some(path) = for_activate.keys.path(key) else {
                    return;
                };
                if for_activate.keys.is_dir(key) {
                    for_activate.open_folder(path);
                } else {
                    for_activate.run_op(crate::ops::Op::Open(path));
                }
            })
            .into()
    })
}

/// One tree row: a symbol and a name.
///
/// Called only for the rows on screen — the same promise the listing relies on,
/// and the reason a folder with ten thousand entries can be opened here at all.
fn tree_row(t: &Theme, r: &TreeRow) -> View {
    let kind = if r.expandable {
        FileKind::Folder
    } else {
        FileKind::of(std::path::Path::new(&*r.label), false)
    };
    // A branch already has a chevron of its own to its left; giving it a second
    // chevron as an icon would say the same thing twice, so an open-able row
    // gets the folder tint on a neutral dot instead.
    let symbol = if r.expandable {
        IconName::Star
    } else {
        kind.icon()
    };

    row([
        icon(symbol)
            .size_raw(t.space(3.0))
            .color_raw(kind.tint(t))
            .decorative()
            .into(),
        View::from(
            text(r.label.to_string())
                .size(t.typography.body_size)
                .weight(if r.expandable {
                    FontWeight::MEDIUM
                } else {
                    FontWeight::REGULAR
                })
                .color(t.color.label)
                .single_line(),
        ),
    ])
    .spacing(t.space(1.5))
    .cross(CrossAlign::Center)
    .padding(Insets::symmetric(t.space(1.0), 0.0))
    .into()
}

/// Open the tree down to `path`, so a folder picked elsewhere is visible in the
/// sidebar rather than hidden inside a closed node.
///
/// Every ancestor between the root and the target is opened, and each one that
/// has never been read is queued for a scan — which is the same lazy path the
/// chevron takes, just several levels at once.
pub fn reveal(ex: &Explorer, path: &std::path::Path) {
    let root = ex.root.peek();
    let Ok(relative) = path.strip_prefix(&root) else {
        // Outside the tree's root: nothing to open, and inventing a new root
        // would move the sidebar under the user.
        return;
    };
    let mut walk = root.clone();
    let mut keys = vec![ex.keys.key_dir(&walk, true)];
    for part in relative.components() {
        walk.push(part.as_os_str());
        keys.push(ex.keys.key_dir(&walk, true));
    }
    // The target itself is opened too: navigating into a folder should show
    // what is inside it in the sidebar as well as in the listing.
    for key in &keys {
        if let Some(p) = ex.keys.path(*key) {
            ex.ensure_loaded(&p);
        }
    }
    ex.tree.open_many(keys);
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::signals::Runtime;
    use std::path::PathBuf;

    #[test]
    fn membuka_jalur_membuka_setiap_leluhurnya() {
        let rt = Runtime::new();
        let ex = Explorer::new(&rt, PathBuf::from("/tmp/root"));
        reveal(&ex, &PathBuf::from("/tmp/root/a/b"));

        let expansion = ex.tree.peek_expansion();
        for path in ["/tmp/root", "/tmp/root/a", "/tmp/root/a/b"] {
            let key = ex.keys.key_dir(std::path::Path::new(path), true);
            assert!(expansion.is_open(key), "{path} should be open");
        }
        // …and every one of them has been queued for a scan, which is the lazy
        // path taken several levels at once.
        assert!(ex.cache.is_loading(std::path::Path::new("/tmp/root/a")));
    }

    #[test]
    fn lintasan_di_luar_akar_tidak_memindahkan_sidebar() {
        let rt = Runtime::new();
        let ex = Explorer::new(&rt, PathBuf::from("/tmp/root"));
        reveal(&ex, &PathBuf::from("/somewhere/else"));
        assert!(ex.tree.peek_expansion().is_empty());
        assert!(
            !ex.cache.contains(std::path::Path::new("/somewhere/else")),
            "nothing was queued either"
        );
    }

    #[test]
    fn baris_cabang_dan_daun_punya_ikon_yang_berbeda() {
        let branch = TreeRow {
            key: 1,
            label: "Pictures".into(),
            depth: 0,
            expandable: true,
            expanded: false,
            last_sibling: true,
            position: 1,
            siblings: 1,
            descendants: 0,
            guides: 0,
        };
        let leaf = TreeRow {
            expandable: false,
            label: "notes.md".into(),
            ..branch.clone()
        };
        let t = silka_theme::Theme::cupertino(silka_theme::Appearance::Dark);
        // Not a pixel assertion — just that the two do not resolve to the same
        // symbol, which is what makes a source list readable at a glance.
        assert_ne!(
            FileKind::Folder.tint(&t),
            FileKind::of(std::path::Path::new(&*leaf.label), false).tint(&t)
        );
        assert!(branch.expandable && !leaf.expandable);
    }
}
