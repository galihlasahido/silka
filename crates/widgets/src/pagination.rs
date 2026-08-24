//! `pagination()` — page N of M, with Previous/Next and a compact number
//! range (`KOMPONEN.md` Tier 3, shadcn Pagination).
//!
//! ```
//! # use silka_core::signals::Runtime;
//! use silka_widgets::pagination;
//!
//! # let rt = Runtime::new();
//! let page = rt.signal(1usize);
//!
//! let nav = pagination(page.get(), 42)
//!     .label("Search results")
//!     .on_change(move |p| page.set(p));
//! # let _ = nav;
//! ```
//!
//! # Built from `button`, not a new render node
//!
//! Every other navigation control in the catalogue ([`mod@crate::breadcrumb`],
//! [`mod@crate::segmented_control`], [`mod@crate::tabs`]) owns a custom
//! [`silka_core::tree::RenderNode`] because each one needs something
//! `button()` does not have: a thumb that slides, a drag gesture, a shrink
//! algorithm for a narrow window. Pagination needs none of that — a page
//! number is a plain pressable thing, and [`crate::button()`] and
//! [`crate::icon_button()`] already are one, with a spring, a focus ring, a
//! hit target and an a11y node built in. Writing that machinery a second time
//! here would be the wrong kind of thoroughness. What this module adds is
//! the assembly and [`page_range`] — the one piece of real logic, and a pure
//! function precisely so it can be tested without a tree, a theme or a GPU.
//!
//! Previous/Next use [`crate::chevron_back`]/[`crate::chevron_forward`]
//! rather than `ChevronLeft`/`ChevronRight` directly, so the arrows already
//! point the right way in a right-to-left layout (§9.8) without this module
//! having to know that.
//!
//! # Which numbers show
//!
//! [`Pagination::siblings`] pages surround the current one, and
//! [`Pagination::boundaries`] pages are always visible at each end; the gap
//! between the two collapses into a single `…`. A gap of exactly one page is
//! **not** collapsed — `1 2 3` and `1 … 3` cost the same width, and the
//! number is more useful than the punctuation. [`page_range`] is the pure
//! function this falls out of; see its doc for the worked examples.
//!
//! # The current page is a toggle, not quite honestly
//!
//! [`crate::Button::toggled`] exists for a formatting toolbar's "bold" —
//! genuinely on or off — and a page number is closer to "the one you are
//! standing on" than to a switch. It is reused here anyway: `toggled` is the
//! only selected-state [`crate::button()`] exposes, screen readers already
//! render it sensibly ("button, pressed"), and inventing a second announcement
//! for the same idea would cost more in surface area than the mismatch costs
//! in precision.
//!
//! # Definition of Done (`KOMPONEN.md`)
//!
//! | Item | Where |
//! |---|---|
//! | Correct in both presets | every colour and size is [`crate::button()`]'s or [`crate::icon_button()`]'s; nothing is drawn in this module |
//! | Interactive states via springs | inherited — the hover/press tint on every button and icon button |
//! | Full keyboard + focus ring | inherited — **each** page is its own Tab stop, which is the correct model here (contrast [`mod@crate::segmented_control`], one control = one stop) |
//! | AccessKit nodes | a [`AccessRole::Button`] per page (`toggled` on the current one) and per Previous/Next, wrapped in one [`AccessRole::Group`] naming the whole control |
//! | Dark mode | tokens only, inherited |
//! | Hit target ≥ 44pt | inherited from [`crate::button()`]/[`crate::icon_button()`] |
//! | Reduced motion | inherited — the underlying springs already honour [`silka_core::animation::Tick`] |
//!
//! # Deliberately not here yet
//!
//! - **Jump to page** (a small `text_field` for typing a page number
//!   directly) — waiting on nothing technical, just scope.
//! - **Page size** (`10 / 25 / 50 per page`) is a `select`, not a `pagination`
//!   concern; a table composes the two rather than this module growing a
//!   second responsibility.
//! - **Swipe/drag** — unlike a segmented control, moving between pages one at
//!   a time by dragging is not an established gesture for this control.

use silka_core::access::AccessRole;
use silka_core::signals::Key;
use silka_core::tree::CrossAlign;
use silka_core::view::{interactive, row, View};
use silka_theme::Theme;

use crate::button::{button_variant_in, ButtonVariant};
use crate::fonts::Fonts;
use crate::icon::{chevron_back_in, chevron_forward_in};
use crate::icon_button::icon_button_with_in;
use crate::images::{active_images, Images};
use crate::text::text_in;

// ---------------------------------------------------------------------------
// page_range — the pure logic
// ---------------------------------------------------------------------------

/// One entry in a rendered page range: a real page, or a collapsed run of
/// hidden ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageItem {
    /// A real, pressable page number (1-based).
    Page(usize),
    /// `…` — pages omitted between two shown ones.
    Ellipsis,
}

/// Which pages to draw for `current` of `total`, showing `siblings` pages on
/// each side of the current one and `boundaries` pages at each end.
///
/// A pure function — the reason it exists separately from the widget at all —
/// so every claim below is a doctest, not a screenshot.
///
/// ```
/// use silka_widgets::pagination::{page_range, PageItem::{Ellipsis, Page}};
///
/// // Small enough that collapsing would not save any width: every page shows.
/// assert_eq!(
///     page_range(4, 7, 1, 1),
///     vec![Page(1), Page(2), Page(3), Page(4), Page(5), Page(6), Page(7)],
/// );
///
/// // A page in the middle of a long run: one ellipsis on each side.
/// assert_eq!(
///     page_range(10, 20, 1, 1),
///     vec![Page(1), Ellipsis, Page(9), Page(10), Page(11), Ellipsis, Page(20)],
/// );
///
/// // Near an end: only the far side collapses.
/// assert_eq!(
///     page_range(1, 20, 1, 1),
///     vec![Page(1), Page(2), Ellipsis, Page(20)],
/// );
///
/// // No pages at all is not an error — it is an empty range.
/// assert_eq!(page_range(1, 0, 1, 1), Vec::new());
/// ```
///
/// `current` outside `1..=total` is clamped rather than panicking — the same
/// "one frame ahead of the signal that holds it" tolerance
/// [`crate::SegmentedControl::active_index`] has, for the same reason: a
/// controlled prop can legitimately be stale for one frame.
pub fn page_range(
    current: usize,
    total: usize,
    siblings: usize,
    boundaries: usize,
) -> Vec<PageItem> {
    if total == 0 {
        return Vec::new();
    }
    let current = current.clamp(1, total);

    // Every page this range wants visible, gathered as a plain set: the
    // boundary runs at each end, plus the sibling window around `current`.
    // Cheap regardless of `total` — the loops are bounded by `boundaries` and
    // `siblings`, both small constants, never by the page count itself.
    let mut shown: Vec<usize> = Vec::with_capacity(2 * boundaries + 2 * siblings + 1);
    shown.extend(1..=boundaries.min(total));
    shown.extend((total.saturating_sub(boundaries) + 1)..=total);
    let lo = current.saturating_sub(siblings).max(1);
    let hi = (current + siblings).min(total);
    shown.extend(lo..=hi);
    shown.sort_unstable();
    shown.dedup();

    // A gap of exactly one page is bridged rather than collapsed: showing
    // the one hidden page costs the same width as `…` and reads better.
    let mut bridged: Vec<usize> = Vec::with_capacity(shown.len() + 2);
    for (i, &p) in shown.iter().enumerate() {
        if i > 0 && p == shown[i - 1] + 2 {
            bridged.push(shown[i - 1] + 1);
        }
        bridged.push(p);
    }

    let mut out = Vec::with_capacity(bridged.len() + 2);
    for (i, &p) in bridged.iter().enumerate() {
        if i > 0 && p > bridged[i - 1] + 1 {
            out.push(PageItem::Ellipsis);
        }
        out.push(PageItem::Page(p));
    }
    out
}

// ---------------------------------------------------------------------------
// OnChange
// ---------------------------------------------------------------------------

/// The "go to page `n`" action the app entrusts to the control.
///
/// The same three properties as [`silka_core::Callback`]: cheap to `Clone`,
/// `PartialEq` by identity, and the only thing it may do is write a signal.
#[derive(Clone)]
pub struct OnChange(std::rc::Rc<dyn Fn(usize)>);

impl OnChange {
    /// Wrap a closure.
    pub fn new(f: impl Fn(usize) + 'static) -> Self {
        Self(std::rc::Rc::new(f))
    }

    /// Ask for `page`.
    pub fn call(&self, page: usize) {
        (self.0)(page)
    }
}

impl PartialEq for OnChange {
    fn eq(&self, other: &Self) -> bool {
        std::rc::Rc::ptr_eq(&self.0, &other.0)
    }
}

impl core::fmt::Debug for OnChange {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("OnChange")
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Dart-style builder for a pagination control (§2.5).
///
/// ```
/// use silka_core::signals::Runtime;
/// use silka_theme::{Appearance, Theme};
/// use silka_widgets::{pagination_in, Fonts};
///
/// let fonts = Fonts::bundled_only();
/// let theme = Theme::cupertino(Appearance::Dark);
/// let rt = Runtime::new();
/// let page = rt.signal(3usize);
///
/// let nav = pagination_in(&fonts, &theme, page.get(), 50)
///     .siblings(2)
///     .boundaries(1)
///     .on_change(move |p| page.set(p));
/// # let _ = nav;
/// ```
pub struct Pagination {
    fonts: Fonts,
    images: Images,
    theme: Theme,
    current: usize,
    total: usize,
    siblings: usize,
    boundaries: usize,
    label: Option<String>,
    previous_label: String,
    next_label: String,
    on_change: Option<OnChange>,
    key: Option<Key>,
}

/// A pagination control for `total` pages, currently on `current` (1-based) —
/// `pagination` (`KOMPONEN.md` Tier 3).
///
/// ```
/// # use silka_core::signals::Runtime;
/// use silka_widgets::pagination;
///
/// # let rt = Runtime::new();
/// let page = rt.signal(1usize);
/// let nav = pagination(page.get(), 10).on_change(move |p| page.set(p));
/// # let _ = nav;
/// ```
///
/// Use [`pagination_in`] outside a build pass.
pub fn pagination(current: usize, total: usize) -> Pagination {
    pagination_in(
        &crate::active_fonts(),
        &crate::ambient::active_theme(),
        current,
        total,
    )
}

/// [`pagination`] with the text engine and theme passed explicitly.
pub fn pagination_in(fonts: &Fonts, theme: &Theme, current: usize, total: usize) -> Pagination {
    Pagination {
        fonts: fonts.clone(),
        images: active_images(),
        theme: *theme,
        current,
        total,
        siblings: 1,
        boundaries: 1,
        label: None,
        previous_label: "Previous".to_string(),
        next_label: "Next".to_string(),
        on_change: None,
        key: None,
    }
}

impl Pagination {
    /// What runs when the user asks for a different page.
    pub fn on_change(mut self, f: impl Fn(usize) + 'static) -> Self {
        self.on_change = Some(OnChange::new(f));
        self
    }

    /// How many pages surround the current one — 1 by default.
    pub fn siblings(mut self, count: usize) -> Self {
        self.siblings = count;
        self
    }

    /// How many pages are always visible at each end — 1 by default.
    pub fn boundaries(mut self, count: usize) -> Self {
        self.boundaries = count;
        self
    }

    /// The whole control's name for screen readers ("Pagination" by default).
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The Previous button's accessible name and, currently, its only text
    /// ("Previous" by default).
    pub fn previous_label(mut self, label: impl Into<String>) -> Self {
        self.previous_label = label.into();
        self
    }

    /// The Next button's accessible name ("Next" by default).
    pub fn next_label(mut self, label: impl Into<String>) -> Self {
        self.next_label = label.into();
        self
    }

    /// The images atlas used to rasterise the chevrons.
    pub fn images(mut self, images: &Images) -> Self {
        self.images = images.clone();
        self
    }

    /// Identity key among its siblings (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The current page, clamped to `1..=total` (or `0` when `total == 0`) —
    /// the value this control actually renders around.
    ///
    /// Never panics on an out-of-range `current`: a page one frame ahead of
    /// the signal that holds it is normal, not an application bug (the same
    /// tolerance [`crate::SegmentedControl::active_index`] has).
    pub fn active_page(&self) -> usize {
        if self.total == 0 {
            0
        } else {
            self.current.clamp(1, self.total)
        }
    }
}

impl From<Pagination> for View {
    fn from(p: Pagination) -> View {
        let t = &p.theme;
        let current = p.active_page();
        let gap = t.space(1.0);

        let mut children: Vec<View> = Vec::new();

        let on_change = p.on_change.clone();
        let previous_target = current.saturating_sub(1).max(1);
        children.push(View::from(
            icon_button_with_in(t, chevron_back_in(&p.images, t), p.previous_label.clone())
                .on_press({
                    let on_change = on_change.clone();
                    move || {
                        if let Some(cb) = &on_change {
                            cb.call(previous_target);
                        }
                    }
                })
                .disabled(current <= 1)
                .key("previous"),
        ));

        let mut ellipsis_seen = 0u32;
        for item in page_range(current, p.total, p.siblings, p.boundaries) {
            match item {
                PageItem::Page(n) => {
                    let selected = n == current;
                    let variant = if selected {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Ghost
                    };
                    let on_change = on_change.clone();
                    children.push(View::from(
                        button_variant_in(&p.fonts, t, n.to_string(), variant)
                            .toggled(selected)
                            .on_press(move || {
                                if let Some(cb) = &on_change {
                                    cb.call(n);
                                }
                            })
                            .key(n),
                    ));
                }
                PageItem::Ellipsis => {
                    // Not a button: nothing to press, nothing for a screen
                    // reader to announce (the default role, `Container`, is
                    // filtered out of the a11y tree entirely).
                    let key = if ellipsis_seen == 0 {
                        "ellipsis-start"
                    } else {
                        "ellipsis-end"
                    };
                    ellipsis_seen += 1;
                    children.push(View::from(
                        text_in(&p.fonts, "…")
                            .size(t.typography.body_size)
                            .color(t.color.tertiary_label)
                            .single_line()
                            .key(key),
                    ));
                }
            }
        }

        let next_target = (current + 1).min(p.total.max(1));
        children.push(View::from(
            icon_button_with_in(t, chevron_forward_in(&p.images, t), p.next_label.clone())
                .on_press({
                    let on_change = on_change.clone();
                    move || {
                        if let Some(cb) = &on_change {
                            cb.call(next_target);
                        }
                    }
                })
                .disabled(current == 0 || current >= p.total)
                .key("next"),
        ));

        let nav = row(children).spacing(gap).cross(CrossAlign::Center);

        // Not focusable itself — the buttons inside are the Tab stops — but
        // named as one group so a screen reader announces "Pagination" (or
        // the app's own label) before the buttons rather than after all of
        // them individually.
        let mut group = interactive(nav)
            .role(AccessRole::Group)
            .label(p.label.unwrap_or_else(|| "Pagination".to_string()))
            .focusable(false);
        if let Some(key) = p.key {
            group = group.key(key);
        }
        group.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silka_core::access::AccessToggled;
    use silka_core::input::{Event, InputRouter, PointerButton, PointerEvent, PointerPhase};
    use silka_core::tree::{BoxConstraints, RenderTree};
    use silka_core::view::reconcile;
    use silka_paint::Size;
    use silka_theme::{Appearance, Preset};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    const BOX: Size = Size::new(600.0, 60.0);

    fn theme() -> Theme {
        Theme::cupertino(Appearance::Dark)
    }

    fn built(view: impl Into<View>) -> RenderTree {
        let mut tree = RenderTree::new();
        reconcile(&mut tree, view);
        tree.layout(BoxConstraints::loose(BOX));
        tree
    }

    // -- page_range ----------------------------------------------------------

    #[test]
    fn rentang_kecil_menampilkan_semua_halaman_tanpa_elipsis() {
        use PageItem::Page;
        // Cukup kecil sehingga batas + tetangga sudah menutup seluruh
        // rentang bahkan dari halaman pertama — tidak ada yang perlu dilipat.
        assert_eq!(page_range(1, 3, 1, 1), vec![Page(1), Page(2), Page(3)]);
    }

    #[test]
    fn celah_satu_halaman_dijembatani_bukan_dilipat() {
        use PageItem::Page;
        // 1 … 3 dan 1 2 3 sama lebar; angkanya lebih berguna dari tanda titik.
        assert_eq!(
            page_range(3, 5, 0, 1),
            vec![Page(1), Page(2), Page(3), Page(4), Page(5)]
        );
    }

    #[test]
    fn halaman_di_tengah_rentang_panjang_dapat_dua_elipsis() {
        use PageItem::{Ellipsis, Page};
        assert_eq!(
            page_range(50, 100, 2, 1),
            vec![
                Page(1),
                Ellipsis,
                Page(48),
                Page(49),
                Page(50),
                Page(51),
                Page(52),
                Ellipsis,
                Page(100)
            ]
        );
    }

    #[test]
    fn dekat_ujung_kiri_hanya_sisi_kanan_yang_melipat() {
        use PageItem::{Ellipsis, Page};
        assert_eq!(
            page_range(1, 100, 1, 1),
            vec![Page(1), Page(2), Ellipsis, Page(100)]
        );
    }

    #[test]
    fn dekat_ujung_kanan_hanya_sisi_kiri_yang_melipat() {
        use PageItem::{Ellipsis, Page};
        assert_eq!(
            page_range(100, 100, 1, 1),
            vec![Page(1), Ellipsis, Page(99), Page(100)]
        );
    }

    #[test]
    fn nol_halaman_adalah_rentang_kosong_bukan_galat() {
        assert_eq!(page_range(1, 0, 1, 1), Vec::new());
    }

    #[test]
    fn satu_halaman_tidak_menghasilkan_elipsis() {
        use PageItem::Page;
        assert_eq!(page_range(1, 1, 1, 1), vec![Page(1)]);
    }

    #[test]
    fn halaman_saat_ini_di_luar_jangkauan_dijepit_bukan_panik() {
        use PageItem::{Ellipsis, Page};
        assert_eq!(
            page_range(999, 20, 1, 1),
            vec![Page(1), Ellipsis, Page(19), Page(20)]
        );
        assert_eq!(
            page_range(0, 20, 1, 1),
            vec![Page(1), Page(2), Ellipsis, Page(20)]
        );
    }

    #[test]
    fn boundaries_nol_tidak_menyertakan_ujung() {
        use PageItem::{Ellipsis, Page};
        assert_eq!(page_range(10, 20, 1, 0), vec![Page(9), Page(10), Page(11)]);
        let _ = Ellipsis;
    }

    // -- widget ----------------------------------------------------------------

    /// One full click through the input layer: move, press, release — the
    /// same three-step contract [`crate::button`]'s own tests use.
    fn klik(router: &mut InputRouter, tree: &mut RenderTree, p: silka_paint::Point) {
        for e in [
            PointerEvent::new(PointerPhase::Move, p, Duration::ZERO),
            PointerEvent::new(PointerPhase::Down, p, Duration::from_millis(8))
                .button(PointerButton::Primary),
            PointerEvent::new(PointerPhase::Up, p, Duration::from_millis(60))
                .button(PointerButton::Primary),
        ] {
            router.dispatch(tree, &Event::Pointer(e));
        }
    }

    /// Click the button whose accessible name is `label`.
    fn klik_label(tree: &mut RenderTree, label: &str) {
        let titik = {
            let a11y = tree.access_tree(None);
            a11y.find_label(label)
                .unwrap_or_else(|| panic!("tidak ada kontrol bernama {label:?}:\n{}", a11y.dump()))
                .bounds
                .center()
        };
        let mut router = InputRouter::new();
        klik(&mut router, tree, titik);
    }

    #[test]
    fn grup_bernama_pagination_secara_default() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(pagination_in(&fonts, &t, 1, 10));
        let a11y = tree.access_tree(None);
        let grup = a11y
            .entries()
            .iter()
            .find(|e| e.node.role == AccessRole::Group)
            .expect("grup pagination ada di pohon a11y");
        assert_eq!(grup.node.label.as_deref(), Some("Pagination"));
    }

    #[test]
    fn label_kustom_menggantikan_bawaan() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(pagination_in(&fonts, &t, 1, 10).label("Search results"));
        let a11y = tree.access_tree(None);
        let grup = a11y
            .entries()
            .iter()
            .find(|e| e.node.role == AccessRole::Group)
            .expect("grup pagination ada di pohon a11y");
        assert_eq!(grup.node.label.as_deref(), Some("Search results"));
    }

    #[test]
    fn menekan_nomor_halaman_memanggil_on_change() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let dipanggil = Rc::new(RefCell::new(None));
        let d = dipanggil.clone();
        let mut tree = built(pagination_in(&fonts, &t, 1, 20).on_change(move |p| {
            *d.borrow_mut() = Some(p);
        }));

        klik_label(&mut tree, "2");

        assert_eq!(*dipanggil.borrow(), Some(2));
    }

    #[test]
    fn menekan_ulang_halaman_saat_ini_tetap_memanggil_on_change() {
        // Bukan `toggled` dalam arti tombol format (bold/italic): menekan
        // halaman yang sedang aktif tetap sah, callback-nya idempoten di sisi
        // aplikasi (lihat catatan di kepala modul).
        let fonts = Fonts::bundled_only();
        let t = theme();
        let dipanggil = Rc::new(RefCell::new(None));
        let d = dipanggil.clone();
        let mut tree = built(pagination_in(&fonts, &t, 3, 20).on_change(move |p| {
            *d.borrow_mut() = Some(p);
        }));

        klik_label(&mut tree, "3");

        assert_eq!(*dipanggil.borrow(), Some(3));
    }

    #[test]
    fn previous_nonaktif_di_halaman_pertama() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let dipanggil = Rc::new(RefCell::new(false));
        let d = dipanggil.clone();
        let mut tree = built(pagination_in(&fonts, &t, 1, 20).on_change(move |_| {
            *d.borrow_mut() = true;
        }));

        klik_label(&mut tree, "Previous");

        assert!(
            !*dipanggil.borrow(),
            "Previous di halaman pertama seharusnya tidak melakukan apa pun"
        );
    }

    #[test]
    fn next_nonaktif_di_halaman_terakhir() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let dipanggil = Rc::new(RefCell::new(false));
        let d = dipanggil.clone();
        let mut tree = built(pagination_in(&fonts, &t, 5, 5).on_change(move |_| {
            *d.borrow_mut() = true;
        }));

        klik_label(&mut tree, "Next");

        assert!(
            !*dipanggil.borrow(),
            "Next di halaman terakhir seharusnya tidak melakukan apa pun"
        );
    }

    #[test]
    fn previous_dan_next_pindah_satu_halaman() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let dipanggil = Rc::new(RefCell::new(None));
        let d = dipanggil.clone();
        let mut tree = built(pagination_in(&fonts, &t, 5, 10).on_change(move |p| {
            *d.borrow_mut() = Some(p);
        }));
        klik_label(&mut tree, "Next");
        assert_eq!(*dipanggil.borrow(), Some(6));

        let dipanggil = Rc::new(RefCell::new(None));
        let d = dipanggil.clone();
        let mut tree = built(pagination_in(&fonts, &t, 5, 10).on_change(move |p| {
            *d.borrow_mut() = Some(p);
        }));
        klik_label(&mut tree, "Previous");
        assert_eq!(*dipanggil.borrow(), Some(4));
    }

    #[test]
    fn setiap_tombol_adalah_titik_fokus_sendiri() {
        // Berbeda dari `segmented_control`: pagination bukan satu kontrol
        // dengan satu titik fokus — Previous, tiap halaman, dan Next masing-
        // masing adalah Tab stop-nya sendiri.
        // Halaman tengah (2 dari 3), supaya Previous **dan** Next dua-duanya
        // aktif — sebuah kontrol nonaktif dikecualikan dari urutan fokus.
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(pagination_in(&fonts, &t, 2, 3));
        let a11y = tree.access_tree(None);
        // Previous, 1, 2, 3, Next — lima kontrol pressable, lima titik fokus.
        assert_eq!(a11y.focus_order().count(), 5, "{}", a11y.dump());
    }

    #[test]
    fn halaman_aktif_diumumkan_lewat_toggled() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(pagination_in(&fonts, &t, 2, 5));
        let a11y = tree.access_tree(None);
        let dua = a11y.find_label("2").expect("halaman 2 ada");
        let tiga = a11y.find_label("3").expect("halaman 3 ada");
        assert_eq!(dua.node.toggled, Some(AccessToggled::On));
        assert_eq!(tiga.node.toggled, Some(AccessToggled::Off));
    }

    #[test]
    fn benar_di_kedua_preset_dan_kedua_penampilan() {
        let fonts = Fonts::bundled_only();
        for preset in Preset::ALL {
            for appearance in [Appearance::Light, Appearance::Dark] {
                let t = Theme::new(preset, appearance);
                let tree = built(pagination_in(&fonts, &t, 4, 9));
                let a11y = tree.access_tree(None);
                // page_range(4, 9, 1, 1) = 1 2 3 4 5 … 9 — six pages plus
                // Previous/Next.
                assert_eq!(
                    a11y.focus_order().count(),
                    8,
                    "{preset:?}/{appearance:?}: {}",
                    a11y.dump()
                );
            }
        }
    }

    #[test]
    fn nol_halaman_tetap_menampilkan_previous_dan_next_nonaktif() {
        let fonts = Fonts::bundled_only();
        let t = theme();
        let tree = built(pagination_in(&fonts, &t, 1, 0));
        let a11y = tree.access_tree(None);
        // Tidak ada nomor halaman yang berarti untuk 0 halaman, tapi Previous
        // dan Next tetap ada di pohon — keduanya cuma nonaktif.
        assert!(a11y.find_label("Previous").is_some());
        assert!(a11y.find_label("Next").is_some());
        assert!(a11y.find_label("1").is_none());
    }

    #[test]
    fn kunci_setiap_tombol_stabil_lintas_rebuild() {
        // `total` cukup kecil sehingga rentang halaman yang tampil sama
        // persis di kedua `current` — satu-satunya yang berubah adalah
        // *tombol mana* yang `toggled`, bukan susunan tombolnya. Kalau kunci
        // tidak stabil, view-diff akan membangun ulang tombolnya, bukan
        // sekadar memindahkan state `toggled`.
        let fonts = Fonts::bundled_only();
        let t = theme();
        assert_eq!(page_range(1, 3, 1, 1), page_range(2, 3, 1, 1));

        let mut tree = RenderTree::new();
        reconcile(&mut tree, pagination_in(&fonts, &t, 1, 3));
        tree.layout(BoxConstraints::loose(BOX));

        let stats = reconcile(&mut tree, pagination_in(&fonts, &t, 2, 3));
        tree.layout(BoxConstraints::loose(BOX));

        assert_eq!(
            stats.created, 0,
            "tidak ada node yang seharusnya dibangun ulang saat halaman terpilih berubah"
        );
    }
}
