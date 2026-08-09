//! Kosakata node aksesibilitas: peran, aksi, dan isi yang **diisi widget**.
//!
//! Kosakata ini milik kita sendiri dan dipetakan 1:1 ke `accesskit` di
//! [`super::bridge`] — persis pola yang dipakai `rustui-paint` terhadap wgpu
//! (§3.2): kode widget tidak pernah menyentuh tipe pustaka luar, sehingga
//! pustaka itu bisa diganti/ditunda tanpa menyentuh satu pun widget.
//!
//! Yang **tidak** ada di sini adalah `bounds` dan daftar anak. Keduanya tidak
//! pernah boleh diisi widget karena hanya hasil layout yang tahu kebenarannya;
//! tempatnya di [`super::AccessEntry`], bukan di [`AccessNode`]. Aturan itu
//! ditegakkan oleh tipe, bukan oleh komentar.

use core::fmt;

/// Peran sebuah node bagi teknologi bantu (screen reader).
///
/// `#[non_exhaustive]`: daftar peran tumbuh mengikuti `KOMPONEN.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum AccessRole {
    /// Wadah struktural murni (padding, align, constrained box).
    ///
    /// Node ini **disaring keluar** dari pohon yang dilihat teknologi bantu —
    /// anak-anaknya naik menggantikannya. Inilah default: sebuah node yang
    /// lupa menyebut perannya tidak akan pernah membuat screen reader
    /// membacakan wadah kosong.
    #[default]
    Container,
    /// Jendela atau akar pohon.
    Window,
    /// Pengelompokan yang berarti (row/column/stack, fieldset).
    Group,
    /// Teks statis.
    Label,
    /// Tombol yang bisa ditekan.
    Button,
    /// Tautan.
    Link,
    /// Kolom teks satu baris.
    TextInput,
    /// Kolom teks multi-baris.
    MultilineTextInput,
    /// Kotak centang (bisa `Mixed`/indeterminate).
    CheckBox,
    /// Tombol radio.
    RadioButton,
    /// Sakelar on/off.
    Switch,
    /// Penggeser nilai.
    Slider,
    /// Penambah/pengurang nilai berundak.
    Stepper,
    /// Wadah yang bisa digulir.
    ScrollView,
    /// Gambar/ikon bermakna.
    Image,
    /// Daftar.
    List,
    /// Satu baris daftar.
    ListItem,
    /// Satu tab.
    Tab,
    /// Deretan tab.
    TabList,
    /// Dialog modal.
    Dialog,
    /// Menu.
    Menu,
    /// Satu item menu.
    MenuItem,
    /// Indikator progres.
    ProgressIndicator,
    /// Garis pemisah.
    Separator,
    /// Toolbar.
    Toolbar,
    /// Tooltip.
    Tooltip,
    /// Tabel.
    Table,
    /// Baris tabel.
    Row,
    /// Sel tabel.
    Cell,
}

impl AccessRole {
    /// Nama pendek untuk tree dump dan inspector.
    pub const fn name(self) -> &'static str {
        match self {
            AccessRole::Container => "container",
            AccessRole::Window => "window",
            AccessRole::Group => "group",
            AccessRole::Label => "label",
            AccessRole::Button => "button",
            AccessRole::Link => "link",
            AccessRole::TextInput => "text_input",
            AccessRole::MultilineTextInput => "text_area",
            AccessRole::CheckBox => "checkbox",
            AccessRole::RadioButton => "radio",
            AccessRole::Switch => "switch",
            AccessRole::Slider => "slider",
            AccessRole::Stepper => "stepper",
            AccessRole::ScrollView => "scroll_view",
            AccessRole::Image => "image",
            AccessRole::List => "list",
            AccessRole::ListItem => "list_item",
            AccessRole::Tab => "tab",
            AccessRole::TabList => "tab_list",
            AccessRole::Dialog => "dialog",
            AccessRole::Menu => "menu",
            AccessRole::MenuItem => "menu_item",
            AccessRole::ProgressIndicator => "progress",
            AccessRole::Separator => "separator",
            AccessRole::Toolbar => "toolbar",
            AccessRole::Tooltip => "tooltip",
            AccessRole::Table => "table",
            AccessRole::Row => "row",
            AccessRole::Cell => "cell",
        }
    }

    /// Benar bila peran ini hanya struktur dan sebaiknya disaring teknologi
    /// bantu (padanan `GenericContainer` AccessKit / `role="none"` ARIA).
    pub const fn is_structural(self) -> bool {
        matches!(self, AccessRole::Container)
    }
}

impl fmt::Display for AccessRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

// ---------------------------------------------------------------------------
// Aksi
// ---------------------------------------------------------------------------

/// Satu aksi yang **diminta** teknologi bantu terhadap sebuah node.
///
/// Bedanya dengan [`AccessActions`]: yang ini adalah permintaan tunggal yang
/// masuk (VoiceOver menekan tombol), sedangkan [`AccessActions`] adalah
/// himpunan kemampuan yang diumumkan node ke luar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AccessAction {
    /// Aktivasi utama (klik tombol, pilih baris).
    Click,
    /// Pindahkan fokus keyboard ke node ini.
    Focus,
    /// Lepaskan fokus keyboard dari node ini.
    Blur,
    /// Naikkan nilai satu langkah (slider, stepper).
    Increment,
    /// Turunkan nilai satu langkah.
    Decrement,
    /// Buka/mekarkan (disclosure, accordion, combo box).
    Expand,
    /// Tutup/lipat.
    Collapse,
    /// Ganti isi (teks yang didikte, nilai numerik).
    SetValue,
    /// Buka menu konteks.
    ShowContextMenu,
    /// Gulir ke atas satu satuan.
    ScrollUp,
    /// Gulir ke bawah satu satuan.
    ScrollDown,
    /// Gulir ke kiri satu satuan.
    ScrollLeft,
    /// Gulir ke kanan satu satuan.
    ScrollRight,
    /// Gulirkan wadah mana pun agar node ini terlihat.
    ScrollIntoView,
}

impl AccessAction {
    /// Kemampuan yang harus diumumkan node agar aksi ini boleh diminta.
    ///
    /// Dipakai untuk **menolak permintaan yang tidak sah** sebelum sampai ke
    /// widget: teknologi bantu bekerja dari snapshot pohon yang bisa saja sudah
    /// satu frame ketinggalan.
    pub const fn capability(self) -> AccessActions {
        match self {
            AccessAction::Click => AccessActions::CLICK,
            AccessAction::Focus | AccessAction::Blur => AccessActions::FOCUS,
            AccessAction::Increment => AccessActions::INCREMENT,
            AccessAction::Decrement => AccessActions::DECREMENT,
            AccessAction::Expand => AccessActions::EXPAND,
            AccessAction::Collapse => AccessActions::COLLAPSE,
            AccessAction::SetValue => AccessActions::SET_VALUE,
            AccessAction::ShowContextMenu => AccessActions::CONTEXT_MENU,
            AccessAction::ScrollUp
            | AccessAction::ScrollDown
            | AccessAction::ScrollLeft
            | AccessAction::ScrollRight
            | AccessAction::ScrollIntoView => AccessActions::SCROLL,
        }
    }

    /// Nama pendek untuk debug/dump.
    pub const fn name(self) -> &'static str {
        match self {
            AccessAction::Click => "click",
            AccessAction::Focus => "focus",
            AccessAction::Blur => "blur",
            AccessAction::Increment => "increment",
            AccessAction::Decrement => "decrement",
            AccessAction::Expand => "expand",
            AccessAction::Collapse => "collapse",
            AccessAction::SetValue => "set_value",
            AccessAction::ShowContextMenu => "context_menu",
            AccessAction::ScrollUp => "scroll_up",
            AccessAction::ScrollDown => "scroll_down",
            AccessAction::ScrollLeft => "scroll_left",
            AccessAction::ScrollRight => "scroll_right",
            AccessAction::ScrollIntoView => "scroll_into_view",
        }
    }
}

impl fmt::Display for AccessAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Permintaan aksi dari teknologi bantu, dalam kosakata kita sendiri.
///
/// Dihasilkan adapter platform setelah dua validasi: node sasaran masih ada di
/// pohon yang terakhir dikirim, **dan** aksinya memang diumumkan node itu.
/// Teknologi bantu bekerja dari snapshot yang bisa satu frame ketinggalan;
/// tanpa validasi itu, klik bisa mendarat di widget yang salah.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessActionRequest {
    /// Node sasaran.
    pub target: crate::tree::NodeId,
    /// Aksi yang diminta.
    pub action: AccessAction,
    /// Isi baru untuk [`AccessAction::SetValue`] (dikte suara, isi ulang field).
    pub value: Option<String>,
}

/// Himpunan kemampuan yang **diumumkan** sebuah node, sebagai bitset.
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct AccessActions(u16);

impl AccessActions {
    /// Tanpa aksi.
    pub const NONE: Self = Self(0);
    /// Bisa diaktifkan (klik/tap/Enter).
    pub const CLICK: Self = Self(1 << 0);
    /// Bisa menerima fokus keyboard.
    pub const FOCUS: Self = Self(1 << 1);
    /// Bisa digulir.
    pub const SCROLL: Self = Self(1 << 2);
    /// Nilainya bisa dinaikkan.
    pub const INCREMENT: Self = Self(1 << 3);
    /// Nilainya bisa diturunkan.
    pub const DECREMENT: Self = Self(1 << 4);
    /// Bisa dimekarkan.
    pub const EXPAND: Self = Self(1 << 5);
    /// Bisa dilipat.
    pub const COLLAPSE: Self = Self(1 << 6);
    /// Isinya bisa diganti langsung (dikte suara, isi ulang field).
    pub const SET_VALUE: Self = Self(1 << 7);
    /// Punya menu konteks.
    pub const CONTEXT_MENU: Self = Self(1 << 8);

    const NAMES: [(Self, &'static str); 9] = [
        (Self::CLICK, "click"),
        (Self::FOCUS, "focus"),
        (Self::SCROLL, "scroll"),
        (Self::INCREMENT, "increment"),
        (Self::DECREMENT, "decrement"),
        (Self::EXPAND, "expand"),
        (Self::COLLAPSE, "collapse"),
        (Self::SET_VALUE, "set_value"),
        (Self::CONTEXT_MENU, "context_menu"),
    ];

    /// Bit mentah.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Benar bila tidak ada aksi sama sekali.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Gabungan dua himpunan aksi.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Benar bila seluruh aksi `other` ada di sini.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Tambahkan aksi.
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// Buang aksi.
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }

    /// Nama tiap bit yang menyala, urut stabil — dipakai tree dump.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        Self::NAMES
            .into_iter()
            .filter(move |(bit, _)| self.contains(*bit))
            .map(|(_, name)| name)
    }
}

impl core::ops::BitOr for AccessActions {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl core::ops::BitOrAssign for AccessActions {
    fn bitor_assign(&mut self, rhs: Self) {
        self.insert(rhs);
    }
}

impl From<AccessAction> for AccessActions {
    fn from(action: AccessAction) -> Self {
        action.capability()
    }
}

impl fmt::Debug for AccessActions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("AccessActions(none)");
        }
        f.write_str("AccessActions(")?;
        for (i, name) in self.names().enumerate() {
            if i > 0 {
                f.write_str("|")?;
            }
            f.write_str(name)?;
        }
        f.write_str(")")
    }
}

// ---------------------------------------------------------------------------
// Node
// ---------------------------------------------------------------------------

/// Keadaan tiga-nilai untuk checkbox/switch/menu item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessToggled {
    /// Tidak aktif.
    Off,
    /// Aktif.
    On,
    /// Sebagian (checkbox indeterminate, `KOMPONEN.md` Tier 2).
    Mixed,
}

impl AccessToggled {
    /// Nama pendek untuk dump.
    pub const fn name(self) -> &'static str {
        match self {
            AccessToggled::Off => "off",
            AccessToggled::On => "on",
            AccessToggled::Mixed => "mixed",
        }
    }
}

impl From<bool> for AccessToggled {
    fn from(v: bool) -> Self {
        if v {
            AccessToggled::On
        } else {
            AccessToggled::Off
        }
    }
}

/// Bagian node aksesibilitas yang **diisi widget**.
///
/// Ini separuh kontrak [`crate::tree::RenderNode::access`]. Separuh lainnya —
/// `bounds`, induk, dan anak — datang dari hasil layout dan dirakit mesin di
/// [`super::AccessEntry`], jadi tidak mungkin basi terhadap yang digambar.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AccessNode {
    /// Peran node.
    pub role: AccessRole,
    /// Nama yang dibacakan screen reader.
    pub label: Option<String>,
    /// Nilai saat ini (isi text field, posisi slider sebagai teks).
    pub value: Option<String>,
    /// Kemampuan yang diumumkan.
    pub actions: AccessActions,
    /// Sembunyikan node **beserta seluruh keturunannya** dari teknologi bantu.
    ///
    /// Untuk dekorasi murni (bayangan, garis hias) dan konten yang sedang
    /// dianimasikan keluar layar.
    pub hidden: bool,
    /// Ada tapi tidak bisa dipakai — tetap dibacakan, dengan status "dimmed".
    pub disabled: bool,
    /// Keadaan on/off/mixed, bila konsepnya berlaku.
    pub toggled: Option<AccessToggled>,
    /// Terpilih atau tidak, bila konsepnya berlaku (baris daftar, baris tabel,
    /// tab, item menu).
    ///
    /// `None` berarti "konsep 'terpilih' tidak berlaku di sini" — dan itu
    /// **bukan** hal yang sama dengan `Some(false)`: node yang mengumumkan
    /// `Some(false)` membuat screen reader membacakan "tidak terpilih" untuk
    /// setiap baris yang dilewati. Karena itu hanya wadah yang memang punya
    /// seleksi yang mengisinya (`AccessKit` mendokumentasikan jebakan yang
    /// sama pada `Selected`).
    pub selected: Option<bool>,
}

impl AccessNode {
    /// Node kosong dengan peran struktural.
    pub fn new() -> Self {
        Self::default()
    }

    /// Node dengan peran tertentu.
    pub fn with_role(role: AccessRole) -> Self {
        Self {
            role,
            ..Self::default()
        }
    }

    /// Setel peran.
    pub fn role(mut self, role: AccessRole) -> Self {
        self.role = role;
        self
    }

    /// Setel nama yang dibacakan.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Setel nilai saat ini.
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Tambahkan kemampuan.
    pub fn with_actions(mut self, actions: AccessActions) -> Self {
        self.actions |= actions;
        self
    }

    /// Tandai tidak bisa dipakai.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Sembunyikan dari teknologi bantu (beserta keturunannya).
    pub fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    /// Setel keadaan on/off/mixed.
    pub fn toggled(mut self, toggled: AccessToggled) -> Self {
        self.toggled = Some(toggled);
        self
    }

    /// Setel keadaan terpilih (baris daftar/tabel, tab, item menu).
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }

    /// Benar bila node bisa menerima fokus keyboard.
    pub fn is_focusable(&self) -> bool {
        self.actions.contains(AccessActions::FOCUS) && !self.disabled
    }
}
