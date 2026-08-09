//! # `tabs` — deretan tab (`KOMPONEN.md` Tier 3)
//!
//! Tiga varian yang diminta katalog, satu mesin: **segmented** (rasa
//! `NSSegmentedControl`), **underline** (rasa shadcn/ui), dan **enclosed**
//! (tab map yang menyatu dengan panelnya). Yang membedakan ketiganya hanyalah
//! token yang diresolusi [`TabsStyle::from_theme`] dan bentuk kotak indikator
//! ([`TabsStyle::indicator_rect`]) — tidak ada satu pun dari mereka yang punya
//! jalur layout, input, atau a11y sendiri.
//!
//! ```
//! # use rustui_core::signals::Runtime;
//! # use rustui_theme::{Appearance, Theme};
//! # use rustui_widgets::Fonts;
//! use rustui_widgets::tabs::{tab, tabs, TabsVariant};
//!
//! # let fonts = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! # let rt = Runtime::new();
//! let terpilih = rt.signal(0usize);
//!
//! let _ = tabs(
//!     &fonts,
//!     &t,
//!     [tab("Umum"), tab("Tampilan"), tab("Lanjutan").disabled(true)],
//! )
//! .variant(TabsVariant::Segmented)
//! .selected(terpilih.get())
//! .label("Pengaturan")
//! .on_select(move |i| terpilih.set(i));
//! ```
//!
//! ## Komponen terkendali
//!
//! `tabs` **tidak** memilih sendiri: `selected` datang dari aplikasi dan
//! `on_select` mengembalikan niat pengguna — pola yang sama dengan `open` pada
//! [`overlay`](mod@crate::overlay). Karena itu isi panel di bawahnya cukup
//! deklaratif: bangun view panel yang aktif saja, dan yang tidak aktif tidak
//! ada di pohon sama sekali (tidak bisa di-Tab, tidak dibacakan screen reader,
//! tidak dilayout).
//!
//! ## Definition of Done (`KOMPONEN.md`)
//!
//! | Butir | Di mana |
//! |---|---|
//! | Benar di kedua preset | [`TabsStyle::from_theme`] — tidak ada satu angka warna pun di luar berkas itu |
//! | State interaktif ber-spring | [`TabBox`] (sorotan hover/press) + [`TabListBox`] (indikator) |
//! | Keyboard penuh + focus ring | Satu deretan = satu perhentian Tab; panah/Home/End di dalamnya, cincin fokus mengelilingi tab aktif |
//! | Node AccessKit | [`AccessRole::TabList`] + [`AccessRole::Tab`] dengan keadaan terpilih |
//! | Dark mode | Ikut token, tanpa satu cabang `if` pun |
//! | Hit target ≥ 44pt | [`TabsStyle::min_height`] dipaksa ke tiap tab saat layout |
//! | Reduced-motion | Indikator [`Essential`](rustui_core::animation::MotionRole::Essential) (kehilangan pantulan), sorotan hover [`Decorative`](rustui_core::animation::MotionRole::Decorative) (hilang sama sekali) |
//!
//! ## Siapa yang mendetakkan spring-nya
//!
//! Sama seperti [`crate::overlay::advance`]: shell memanggil [`advance`] sekali
//! per frame, dan fungsi itu yang menjawab apakah masih ada yang bergerak
//! (§3.5 "render hanya saat dirty"). Selama sambungan
//! [`AnimationDriver`](rustui_core::animation::AnimationDriver) ke siklus frame
//! aplikasi belum ada, sebuah shell bisa saja tidak pernah memanggilnya —
//! dan node di sini **tidak membeku** kalau itu terjadi: sebelum ada satu pun
//! detak, transisi dijalankan sebagai lompatan. Begitu detaknya datang,
//! transisi yang sama menjadi spring tanpa satu baris pun berubah di aplikasi.

pub mod item;
pub mod list;
pub mod style;
#[cfg(test)]
mod tests;

use rustui_core::animation::{Spring, Tick};
use rustui_core::scheduler::Dirty;
use rustui_core::signals::Key;
use rustui_core::tree::{AccessRole, CrossAlign, MainAlign, NodeId, RenderTree};
use rustui_core::view::{row, Builder, View};
use rustui_core::Callback;
use rustui_text::FontWeight;
use rustui_theme::Theme;

use crate::fonts::Fonts;
use crate::text::text;

pub use item::{TabBox, TabProps, TAB_TINT_MOTION};
pub use list::{OnSelect, TabListBox, TabListProps};
pub use style::{TabsStyle, TabsVariant};

// ---------------------------------------------------------------------------
// Satu tab
// ---------------------------------------------------------------------------

/// Deskripsi satu tab: label, keadaan, dan kunci identitasnya.
///
/// Sengaja **bukan** [`View`]: deretan perlu membaca `disabled` sebelum pohon
/// dirakit (navigasi panah melewati tab yang mati), dan begitu sesuatu menjadi
/// `View` propsnya terkubur di balik `dyn ViewNode`. Alasan yang sama membuat
/// [`crate::overlay::OverlayBuilder`] punya tipe sendiri.
#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    label: String,
    disabled: bool,
    key: Option<Key>,
}

/// Satu tab berlabel `label`.
pub fn tab(label: impl Into<String>) -> Tab {
    Tab {
        label: label.into(),
        disabled: false,
        key: None,
    }
}

impl Tab {
    /// Tab yang tidak bisa dipilih (tetap dibacakan sebagai dimmed).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Kunci identitas — wajib untuk daftar tab yang berubah isinya (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Label yang dibacakan screen reader.
    pub fn label_text(&self) -> &str {
        &self.label
    }

    /// Benar bila tab ini tidak bisa dipilih.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Builder deretan tab bergaya Dart (§2.5).
///
/// Tipe sendiri, bukan [`Builder`], karena ia harus **merakit anak-anaknya**
/// dari daftar [`Tab`] pada saat menjadi [`View`]: warna label, tebal huruf,
/// dan callback per-indeks semuanya turunan dari `selected` dan `style` yang
/// baru diketahui setelah seluruh method chain selesai ditulis.
pub struct Tabs {
    fonts: Fonts,
    theme: Theme,
    items: Vec<Tab>,
    variant: TabsVariant,
    style: Option<TabsStyle>,
    equal_widths: Option<bool>,
    selected: usize,
    label: Option<String>,
    on_select: Option<OnSelect>,
    spring: Spring,
    key: Option<Key>,
}

/// Deretan tab berisi `items`.
///
/// `fonts` adalah mesin teks aplikasi dan `theme` sumber seluruh nilainya —
/// tidak ada satu angka pun yang lahir di kode aplikasi (§2.6).
pub fn tabs(fonts: &Fonts, theme: &Theme, items: impl IntoIterator<Item = Tab>) -> Tabs {
    Tabs {
        fonts: fonts.clone(),
        theme: *theme,
        items: items.into_iter().collect(),
        variant: TabsVariant::default(),
        style: None,
        equal_widths: None,
        selected: 0,
        label: None,
        on_select: None,
        // `snappy` adalah preset yang paling dekat dengan rasa memilih segmen
        // di macOS: cepat sampai, sedikit sekali pantulan (WWDC23).
        spring: Spring::snappy(),
        key: None,
    }
}

impl Tabs {
    /// Varian visual (bawaan [`TabsVariant::Segmented`]).
    pub fn variant(mut self, variant: TabsVariant) -> Self {
        self.variant = variant;
        self
    }

    /// Varian [`TabsVariant::Segmented`].
    pub fn segmented(self) -> Self {
        self.variant(TabsVariant::Segmented)
    }

    /// Varian [`TabsVariant::Underline`].
    pub fn underline(self) -> Self {
        self.variant(TabsVariant::Underline)
    }

    /// Varian [`TabsVariant::Enclosed`].
    pub fn enclosed(self) -> Self {
        self.variant(TabsVariant::Enclosed)
    }

    /// Indeks tab yang sedang aktif (komponen terkendali).
    pub fn selected(mut self, index: usize) -> Self {
        self.selected = index;
        self
    }

    /// Apa yang dijalankan saat pengguna memilih tab lain.
    pub fn on_select(mut self, f: impl Fn(usize) + 'static) -> Self {
        self.on_select = Some(OnSelect::new(f));
        self
    }

    /// Nama deretan bagi screen reader.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Paksa semua tab selebar yang terlebar (bawaan: hanya di segmented).
    pub fn equal_widths(mut self, equal: bool) -> Self {
        self.equal_widths = Some(equal);
        self
    }

    /// Spring yang menjalankan indikator dan sorotan (`smooth`/`snappy`/
    /// `bouncy`).
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Ganti seluruh nilai visual sekaligus — escape hatch untuk brand kustom
    /// yang tidak cukup diselesaikan dengan mengganti token theme (§2.7).
    pub fn style(mut self, style: TabsStyle) -> Self {
        self.variant = style.variant;
        self.style = Some(style);
        self
    }

    /// Kunci identitas di antara saudara-saudaranya (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Nilai visual yang akan dipakai — token yang sudah diresolusi.
    pub fn resolved_style(&self) -> TabsStyle {
        let mut style = self
            .style
            .unwrap_or_else(|| TabsStyle::from_theme(&self.theme, self.variant));
        if let Some(equal) = self.equal_widths {
            style.equal_widths = equal;
        }
        style
    }

    /// Indeks aktif yang benar-benar berlaku: dijepit ke daftar yang ada.
    ///
    /// Indeks di luar jangkauan **tidak** panik dan tidak menghilangkan
    /// indikator — daftar tab yang menyusut satu frame lebih dulu daripada
    /// signal pilihannya adalah kejadian normal, bukan bug aplikasi.
    pub fn active_index(&self) -> usize {
        if self.items.is_empty() {
            return 0;
        }
        self.selected.min(self.items.len() - 1)
    }
}

impl From<Tabs> for View {
    fn from(t: Tabs) -> View {
        let style = t.resolved_style();
        let aktif = t.active_index();
        let props = TabListProps {
            style,
            selected: aktif,
            label: t.label.clone(),
            on_select: t.on_select.clone(),
            enabled: t.items.iter().map(|i| !i.disabled).collect(),
            spring: t.spring,
        };

        let mut builder = Builder::new(props);
        for (i, item) in t.items.iter().enumerate() {
            builder = builder.child(tab_view(&t, &style, i, item, i == aktif));
        }
        if let Some(key) = t.key {
            builder = builder.key(key);
        }
        builder.into()
    }
}

/// Rakit satu tab menjadi view: sorotan + label di atas token.
fn tab_view(t: &Tabs, style: &TabsStyle, index: usize, item: &Tab, selected: bool) -> View {
    let warna = if item.disabled {
        style.disabled_label
    } else if selected {
        style.selected_label
    } else {
        style.label
    };

    // Label dibungkus wadah flex yang meratakannya — bukan aritmetika di sini
    // (§3.4). Perannya `Container` supaya screen reader tidak membacakan nama
    // tab dua kali: sekali dari node tab, sekali dari teksnya.
    let isi = row([text(&t.fonts, &item.label)
        .size(style.label_size)
        .weight(if selected {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::MEDIUM
        })
        .color(warna)
        .single_line()
        .role(AccessRole::Container)])
    .main(MainAlign::Center)
    .cross(CrossAlign::Center)
    .padding(style.tab_padding);

    let on_press = t.on_select.clone().map(|cb| {
        Callback::new(move || {
            cb.call(index);
        })
    });

    let mut b = Builder::new(TabProps {
        label: item.label.clone(),
        index,
        selected,
        disabled: item.disabled,
        corners: style.tab_corners,
        hover: style.hover,
        pressed: style.pressed,
        on_press,
        spring: t.spring,
    })
    .child(isi);
    if let Some(key) = item.key.clone() {
        b = b.key(key);
    }
    b.into()
}

// ---------------------------------------------------------------------------
// Detak
// ---------------------------------------------------------------------------

/// Semua node `tabs` di `tree`, urut pre-order.
fn nodes(tree: &RenderTree) -> Vec<NodeId> {
    fn kumpulkan(tree: &RenderTree, id: NodeId, out: &mut Vec<NodeId>) {
        if let Some(node) = tree.render(id) {
            if node.downcast_ref::<TabListBox>().is_some()
                || node.downcast_ref::<TabBox>().is_some()
            {
                out.push(id);
            }
        }
        for anak in tree.children(id) {
            kumpulkan(tree, *anak, out);
        }
    }
    let mut out = Vec::new();
    kumpulkan(tree, tree.root(), &mut out);
    out
}

/// Majukan seluruh transisi `tabs` satu frame.
///
/// Dipanggil shell **sekali per frame**, tanpa syarat: fungsi inilah yang tahu
/// apakah masih ada yang bergerak, dan jawabannya yang menentukan apakah frame
/// berikutnya perlu dijadwalkan (§3.5). Yang dikembalikan:
///
/// - [`Dirty::PAINT`] — ada indikator atau sorotan yang **berubah** frame ini.
/// - [`Dirty::ANIMATION`] — masih ada spring yang belum settle. Begitu bendera
///   ini hilang, GPU boleh tidur.
/// - [`Dirty::NONE`] — tidak ada pekerjaan yang lahir dari modul ini.
///
/// Indikator bergerak **tanpa** memicu layout: posisi tab tidak bergantung
/// padanya, jadi satu deretan yang beranimasi tidak pernah membuat window
/// dihitung ulang.
///
/// ```
/// # use rustui_core::animation::{Motion, Tick};
/// # use rustui_core::scheduler::Dirty;
/// # use rustui_core::tree::{BoxConstraints, RenderTree};
/// # use rustui_core::view::reconcile;
/// # use rustui_paint::Size;
/// # use rustui_theme::{Appearance, Theme};
/// # use rustui_widgets::Fonts;
/// # use std::time::Duration;
/// use rustui_widgets::tabs::{advance, tab, tabs};
///
/// # let fonts = Fonts::bundled_only();
/// # let t = Theme::tailwind(Appearance::Light);
/// let mut tree = RenderTree::new();
/// let tick = Tick::manual(Duration::from_millis(8), Motion::Full);
///
/// reconcile(&mut tree, tabs(&fonts, &t, [tab("Satu"), tab("Dua")]).selected(0));
/// tree.layout(BoxConstraints::tight(Size::new(400.0, 60.0)));
/// // Deretan yang baru lahir sudah pada tempatnya: tidak ada yang bergerak.
/// assert_eq!(advance(&mut tree, &tick), Dirty::NONE);
/// ```
pub fn advance(tree: &mut RenderTree, tick: &Tick) -> Dirty {
    let mut dirty = Dirty::NONE;
    for id in nodes(tree) {
        let (berubah, bergerak) = if let Some(l) = tree.node_mut_ref::<TabListBox>(id) {
            (l.advance(tick), l.is_animating())
        } else if let Some(t) = tree.node_mut_ref::<TabBox>(id) {
            (t.advance(tick), t.is_animating())
        } else {
            continue;
        };
        if berubah {
            tree.mark_needs_paint(id);
            dirty |= Dirty::PAINT;
        }
        if bergerak {
            dirty |= Dirty::ANIMATION;
        }
    }
    dirty
}

/// Benar bila masih ada transisi `tabs` yang berjalan.
pub fn is_animating(tree: &RenderTree) -> bool {
    nodes(tree).into_iter().any(|id| {
        tree.node_ref::<TabListBox>(id)
            .is_some_and(TabListBox::is_animating)
            || tree
                .node_ref::<TabBox>(id)
                .is_some_and(TabBox::is_animating)
    })
}

/// Selesaikan seluruh transisi `tabs` seketika (dipakai uji dan snapshot).
pub fn settle(tree: &mut RenderTree) {
    for id in nodes(tree) {
        if let Some(l) = tree.node_mut_ref::<TabListBox>(id) {
            l.settle();
        } else if let Some(t) = tree.node_mut_ref::<TabBox>(id) {
            t.settle();
        }
        tree.mark_needs_paint(id);
    }
}
