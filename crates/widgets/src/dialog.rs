//! `dialog()` dan `alert()` — komponen Tier 4 pertama (`KOMPONEN.md`).
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # use silka_core::view::fixed;
//! # use silka_theme::{Appearance, Theme};
//! use silka_widgets::{dialog, overlay_layer, Fonts};
//!
//! # let rt = Runtime::new();
//! # let terbuka = rt.signal(true);
//! # let f = Fonts::bundled_only();
//! # let t = Theme::cupertino(Appearance::Dark);
//! overlay_layer(fixed(800.0, 600.0).background(t.color.background)).overlay(
//!     dialog(&f, &t, "Simpan perubahan?")
//!         .message("Perubahan yang belum disimpan akan hilang.")
//!         .open(terbuka.get())
//!         .cancel("Batal", move || terbuka.set(false))
//!         .confirm("Simpan", move || terbuka.set(false)),
//! );
//! ```
//!
//! Catatan khusus `KOMPONEN.md` untuk komponen ini ada dua, dan keduanya
//! dijawab di berkas ini:
//!
//! 1. **"Modal dengan backdrop dim"** — bukan node baru: dialog adalah preset
//!    di atas [`crate::overlay`], yang memang dibangun sekali untuk sepuluh
//!    komponen (aturan pengerjaan #3). Yang dipilih di sini hanyalah
//!    [`Barrier::Modal`], [`Placement::center`], dan backdrop token `scrim`;
//!    geometri, dismiss, perangkap fokus, dan transisi spring-nya sudah ada.
//! 2. **"Tombol default/cancel mengikuti konvensi per-OS"** — [`ButtonOrder`].
//!    Aksi ditulis aplikasi dalam urutan **makna** (konfirmasi, batal, lainnya)
//!    dan susunan visualnya ditentukan platform: di macOS dan GNOME tombol
//!    default berada paling kanan dengan Batal di kirinya, di Windows justru
//!    sebaliknya. Aplikasi tidak pernah menulis `#[cfg(target_os)]` untuk ini.
//!
//! Definition of Done (`KOMPONEN.md`) yang dipenuhi:
//!
//! - **Kedua preset** lewat token semantik — tidak ada satu pun angka warna,
//!   radius, atau jarak yang lahir di sini.
//! - **Transisi spring yang bisa di-retarget**: dialog yang ditutup di tengah
//!   animasi buka berbalik arah membawa kecepatannya ([`crate::overlay`] §3.5).
//! - **Keyboard penuh + focus ring**: Tab terperangkap di dalam panel (modal =
//!   focus scope), Space mengaktifkan kontrol yang terfokus, dan **Esc**
//!   menjalankan aksi batal.
//!
//!   Aturan **Return** ditulis di sini apa adanya karena ia satu-satunya yang
//!   punya dua kemungkinan jawaban: Return diberikan lebih dulu ke node yang
//!   terfokus, jadi tombol yang sedang difokuskan menang atas tombol default
//!   (perilaku shadcn/web). Begitu yang terfokus **tidak** menelan Return —
//!   kolom teks di dalam [`DialogBuilder::content`], atau belum ada yang
//!   terfokus sama sekali — Return menggelembung ke [`DialogPanel`] dan
//!   menjalankan tombol default (perilaku HIG). Yang tidak pernah terjadi:
//!   Return menjalankan aksi merusak.
//! - **Node AccessKit**: panelnya beperan [`AccessRole::Dialog`] dengan judul
//!   sebagai namanya, isinya dibacakan, dan konten di belakang benar-benar
//!   inert.
//! - **Dark mode**, **hit target ≥ 44pt** (tombolnya [`crate::button`]), dan
//!   **reduced-motion** (transisinya [`silka_core::animation::MotionRole`]
//!   `Essential`: pantulan dibuang, gerakan yang menjelaskan dipertahankan).
//!
//! ## Enter tanpa fokus
//!
//! Jalur normal Return adalah menggelembung dari node terfokus ke atas dan
//! melewati [`DialogPanel`]. Tapi bila belum ada satu pun yang terfokus, event
//! tombol hanya sampai ke akar pohon — persis lubang yang sama yang ditambal
//! [`crate::overlay::dismiss_topmost`] untuk Esc. [`activate_default`] adalah
//! pasangannya untuk Return, dan shell memanggilnya dengan syarat yang sama:
//! **hanya** saat router menjawab tidak ada yang menangani.

use silka_core::access::{AccessNode, AccessRole};
use silka_core::animation::Spring;
use silka_core::input::{Event, EventCtx, NamedKey};
use silka_core::scheduler::Dirty;
use silka_core::signals::Key;
use silka_core::tree::{
    BoxConstraints, CrossAlign, LayoutCtx, MainAlign, NodeId, RenderNode, RenderTree,
};
use silka_core::view::{column, constrained, row, Builder, View, ViewNode};
use silka_core::Callback;
use silka_paint::{Insets, Point, Size};
use silka_text::FontWeight;
use silka_theme::{Theme, TypeStyle};

use crate::button::{button_variant, ButtonVariant};
use crate::fonts::Fonts;
use crate::overlay::{overlay, Barrier, Dismiss, OverlayBuilder, OverlayEntry, Placement};
use crate::text::{text, Text};

/// Lebar panel dialog dalam **langkah skala spacing** (§2.6).
///
/// 90 × 4pt = 360pt: di antara `NSAlert` (260pt, terlalu sempit untuk teks
/// penjelas) dan `Dialog` shadcn (512pt, terlalu lebar untuk sebuah alert).
/// Angka ini tetap sebuah kelipatan skala, bukan lebar bebas.
pub const DIALOG_WIDTH_STEPS: f32 = 90.0;

// ---------------------------------------------------------------------------
// Urutan tombol
// ---------------------------------------------------------------------------

/// Susunan tombol dialog — satu-satunya hal di komponen ini yang benar-benar
/// berbeda antar sistem operasi.
///
/// | Platform | Susunan (kiri → kanan) |
/// |---|---|
/// | macOS (HIG), GNOME | `[lainnya…] [Batal] [Default]` |
/// | Windows | `[Default] [Batal] [lainnya…]` |
///
/// Aplikasi menulis aksinya dalam urutan **makna**, bukan urutan piksel;
/// [`ButtonOrder::Platform`] yang menerjemahkannya. Di antarmuka RTL barisnya
/// ikut tercermin dengan sendirinya karena [`row`] mengikuti arah baca (§9.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ButtonOrder {
    /// Ikuti konvensi OS tempat aplikasi ini dikompilasi ([`ButtonOrder::PLATFORM`]).
    #[default]
    Platform,
    /// Tombol default paling akhir (macOS, GNOME).
    ConfirmLast,
    /// Tombol default paling awal (Windows).
    ConfirmFirst,
}

impl ButtonOrder {
    /// Konvensi OS target build ini.
    ///
    /// Ditentukan saat kompilasi, bukan saat jalan: tidak ada aplikasi yang
    /// perlu menanyakan sistem operasinya sendiri hanya untuk menyusun dua
    /// tombol.
    pub const PLATFORM: ButtonOrder = if cfg!(target_os = "windows") {
        ButtonOrder::ConfirmFirst
    } else {
        ButtonOrder::ConfirmLast
    };

    /// Susunan konkret — [`ButtonOrder::Platform`] diganti [`ButtonOrder::PLATFORM`].
    pub fn resolved(self) -> Self {
        match self {
            ButtonOrder::Platform => ButtonOrder::PLATFORM,
            lain => lain,
        }
    }

    /// Susun ulang `actions` menjadi urutan visual.
    ///
    /// Fungsi murni, dan sengaja: inilah satu-satunya bagian "konvensi per-OS"
    /// yang punya jawaban benar/salah, jadi ia harus bisa diuji tanpa pohon,
    /// tanpa GPU, dan untuk **kedua** platform sekaligus (§9.5).
    ///
    /// ```
    /// use silka_widgets::dialog::{action, ButtonOrder};
    ///
    /// let urut = ButtonOrder::ConfirmLast.arrange(vec![
    ///     action("Simpan").confirm(),
    ///     action("Batal").cancel(),
    /// ]);
    /// let nama: Vec<&str> = urut.iter().map(|a| a.label()).collect();
    /// assert_eq!(nama, ["Batal", "Simpan"]);
    /// ```
    pub fn arrange(self, actions: Vec<DialogAction>) -> Vec<DialogAction> {
        // Satu aturan, bukan dua: pisahkan jadi tiga kelompok peran, lalu
        // rangkai kelompoknya sesuai konvensi. Yang bertukar tempat adalah
        // **kelompok**, bukan tombol satu per satu — urutan yang ditulis
        // aplikasi di dalam sebuah kelompok tetap urutan bacanya, di kedua
        // platform. (Membalik seluruh vektor akan ikut menukar dua tombol
        // "lainnya" yang seharusnya tetap berdampingan sesuai penulisan.)
        let mut lainnya: Vec<DialogAction> = Vec::new();
        let mut batal: Vec<DialogAction> = Vec::new();
        let mut utama: Vec<DialogAction> = Vec::new();
        for a in actions {
            match a.kind {
                ActionKind::Plain => lainnya.push(a),
                ActionKind::Cancel => batal.push(a),
                ActionKind::Confirm | ActionKind::Destructive => utama.push(a),
            }
        }

        let mut out: Vec<DialogAction> =
            Vec::with_capacity(lainnya.len() + batal.len() + utama.len());
        if self.resolved() == ButtonOrder::ConfirmFirst {
            // Windows: `[Default] [Batal] [lainnya…]`.
            out.append(&mut utama);
            out.append(&mut batal);
            out.append(&mut lainnya);
        } else {
            // macOS/GNOME: `[lainnya…] [Batal] [Default]`.
            out.append(&mut lainnya);
            out.append(&mut batal);
            out.append(&mut utama);
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Aksi
// ---------------------------------------------------------------------------

/// Peran sebuah tombol dialog — menentukan posisi, varian visual, dan tombol
/// keyboard yang menjalankannya.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ActionKind {
    /// Aksi utama, dijalankan **Return** dari mana pun di dalam dialog.
    Confirm,
    /// Batal: dijalankan **Esc**, dan (bila diizinkan) klik di luar panel.
    Cancel,
    /// Aksi merusak (Hapus, Buang). Menempati posisi yang sama dengan
    /// [`ActionKind::Confirm`] tapi **tidak pernah** menjadi tombol default —
    /// HIG melarang aksi merusak dijalankan tanpa sengaja oleh Return.
    Destructive,
    /// Aksi tambahan tanpa peran khusus ("Jangan Simpan").
    #[default]
    Plain,
}

/// Satu tombol dialog.
///
/// Ditulis gaya Dart (§2.5): [`action`] lalu method chaining.
#[derive(Debug, Clone)]
pub struct DialogAction {
    label: String,
    kind: ActionKind,
    on_press: Option<Callback>,
    disabled: bool,
}

/// Tombol dialog berlabel `label`, tanpa peran khusus.
pub fn action(label: impl Into<String>) -> DialogAction {
    DialogAction {
        label: label.into(),
        kind: ActionKind::Plain,
        on_press: None,
        disabled: false,
    }
}

impl DialogAction {
    /// Jadikan aksi utama (tombol default; dijalankan Return).
    pub fn confirm(mut self) -> Self {
        self.kind = ActionKind::Confirm;
        self
    }

    /// Jadikan aksi batal (dijalankan Esc).
    pub fn cancel(mut self) -> Self {
        self.kind = ActionKind::Cancel;
        self
    }

    /// Jadikan aksi merusak.
    pub fn destructive(mut self) -> Self {
        self.kind = ActionKind::Destructive;
        self
    }

    /// Apa yang dijalankan saat tombol ini diaktifkan.
    pub fn on_press(mut self, f: impl Fn() + 'static) -> Self {
        self.on_press = Some(Callback::new(f));
        self
    }

    /// Matikan tombol ini (tetap dibacakan sebagai dimmed).
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Nama tombol.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Peran tombol.
    pub fn kind(&self) -> ActionKind {
        self.kind
    }

    /// Benar bila tombol ini tidak bisa dipakai.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Varian visual [`crate::button`] untuk peran ini.
    ///
    /// Pemetaan, bukan pilihan warna: seluruh warnanya tetap milik token.
    pub fn variant(&self) -> ButtonVariant {
        match self.kind {
            ActionKind::Confirm => ButtonVariant::Primary,
            ActionKind::Destructive => ButtonVariant::Destructive,
            ActionKind::Cancel | ActionKind::Plain => ButtonVariant::Secondary,
        }
    }

    /// Callback aksi ini, bila ada.
    fn callback(&self) -> Option<Callback> {
        self.on_press.clone().filter(|_| !self.disabled)
    }
}

// ---------------------------------------------------------------------------
// Node panel
// ---------------------------------------------------------------------------

/// Node panel dialog: **satu-satunya alasannya ada adalah tombol default**.
///
/// Selain itu ia transparan — layout diteruskan apa adanya dan perannya
/// struktural, karena nama dan peran dialog sudah diumumkan
/// [`OverlayEntry`] di atasnya (satu dialog = satu nama, bukan dua).
pub struct DialogPanel {
    /// Dialognya sedang terbuka (bukan sedang beranimasi keluar).
    pub open: bool,
    /// Aksi yang dijalankan Return.
    pub default_action: Option<Callback>,
}

impl DialogPanel {
    /// Jalankan tombol default; benar bila memang ada yang dijalankan.
    ///
    /// Callback disalin keluar dulu — ia hampir selalu menulis signal, dan
    /// tulisan signal boleh memicu apa saja; yang tidak boleh adalah ia
    /// berjalan sambil node ini masih dipinjam `&mut` (pola yang sama dengan
    /// [`silka_core::tree::Interactive`]).
    pub fn activate_default(&mut self) -> bool {
        if !self.open {
            return false;
        }
        let Some(cb) = self.default_action.clone() else {
            return false;
        };
        cb.call();
        true
    }
}

impl RenderNode for DialogPanel {
    fn type_name(&self) -> &'static str {
        "DialogPanel"
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, constraints: BoxConstraints) -> Size {
        if ctx.child_count() == 0 {
            return constraints.smallest();
        }
        let anak = ctx.child(0);
        let ukuran = ctx.layout_child(anak, constraints);
        ctx.place_child(anak, Point::ZERO);
        constraints.constrain(ukuran)
    }

    fn access(&self, node: &mut AccessNode) {
        node.role = AccessRole::Container;
    }

    fn event(&mut self, ctx: &mut EventCtx<'_>, event: &Event) {
        // Return hanya ditandai handled kalau dialog ini memang punya tombol
        // default: dialog tanpa aksi utama harus **membiarkan** Return
        // menggelembung, bukan menelannya diam-diam (aturan yang sama dengan
        // Esc di `OverlayEntry`).
        let Event::Key(k) = event else { return };
        if !k.is_pressed() || !k.code.is(NamedKey::Enter) || !k.modifiers.is_empty() {
            return;
        }
        if self.activate_default() {
            ctx.handled();
        }
    }
}

impl core::fmt::Debug for DialogPanel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DialogPanel")
            .field("open", &self.open)
            .field("default_action", &self.default_action.is_some())
            .finish()
    }
}

/// Props [`DialogPanel`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DialogPanelProps {
    open: bool,
    default_action: Option<Callback>,
}

impl ViewNode for DialogPanelProps {
    fn build(&self) -> Box<dyn RenderNode> {
        Box::new(DialogPanel {
            open: self.open,
            default_action: self.default_action.clone(),
        })
    }

    fn update(&self, node: &mut dyn RenderNode) -> Dirty {
        let n = node
            .downcast_mut::<DialogPanel>()
            .expect("tipe view sama berarti tipe render node sama");
        n.open = self.open;
        // Callback selalu diganti tanpa dibandingkan: closure dibangun ulang
        // tiap rebuild dan menangkap nilai baru (lihat `InteractiveProps`).
        n.default_action.clone_from(&self.default_action);
        Dirty::NONE
    }
}

// ---------------------------------------------------------------------------
// Jaring pengaman Return
// ---------------------------------------------------------------------------

/// Jalankan tombol default dialog paling atas; benar bila ada yang dijalankan.
///
/// Pasangan [`crate::overlay::dismiss_topmost`] untuk Return, dengan syarat
/// pemakaian yang sama persis — shell memanggilnya **hanya** saat router
/// menjawab tidak ada yang menangani:
///
/// ```
/// # use silka_core::input::{Event, InputRouter, KeyEvent, KeyCode, NamedKey};
/// # use silka_core::tree::RenderTree;
/// # use std::time::Duration;
/// # use silka_widgets::dialog::activate_default;
/// # let mut tree = RenderTree::new();
/// # let mut router = InputRouter::new();
/// let enter = Event::Key(KeyEvent::pressed(
///     KeyCode::Named(NamedKey::Enter),
///     Duration::ZERO,
/// ));
/// if !router.dispatch(&mut tree, &enter).handled {
///     activate_default(&mut tree);
/// }
/// ```
pub fn activate_default(tree: &mut RenderTree) -> bool {
    let Some(panel) = panel_teratas(tree) else {
        return false;
    };
    tree.node_mut_ref::<DialogPanel>(panel)
        .is_some_and(DialogPanel::activate_default)
}

/// Panel dialog milik overlay terbuka yang paling atas.
fn panel_teratas(tree: &RenderTree) -> Option<NodeId> {
    crate::overlay::entries(tree)
        .into_iter()
        .rev()
        .filter(|id| {
            tree.node_ref::<OverlayEntry>(*id)
                .is_some_and(|o| o.open && o.is_visible())
        })
        .find_map(|id| cari_panel(tree, id))
}

fn cari_panel(tree: &RenderTree, akar: NodeId) -> Option<NodeId> {
    if tree.node_ref::<DialogPanel>(akar).is_some() {
        return Some(akar);
    }
    tree.children(akar)
        .iter()
        .find_map(|anak| cari_panel(tree, *anak))
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Dialog modal berjudul `title` — padanan `Dialog` shadcn.
///
/// Bawaannya bisa ditutup dengan Esc **maupun** klik di luar panel; untuk
/// alert yang tidak boleh hilang tak sengaja, pakai [`alert`].
pub fn dialog(fonts: &Fonts, theme: &Theme, title: impl Into<String>) -> DialogBuilder {
    DialogBuilder {
        fonts: fonts.clone(),
        theme: *theme,
        key: None,
        title: title.into(),
        message: None,
        content: None,
        actions: Vec::new(),
        order: ButtonOrder::default(),
        open: false,
        width: theme.space(DIALOG_WIDTH_STEPS),
        dismiss: Dismiss::ALL,
        on_dismiss: None,
        spring: Spring::snappy(),
    }
}

/// Alert modal — padanan `NSAlert`.
///
/// Bedanya dengan [`dialog`] cuma satu, dan itu bukan soal tampilan: klik di
/// luar panel **tidak** menutupnya. Sebuah alert menanyakan sesuatu yang harus
/// dijawab; menghilangkannya karena kursor tergelincir adalah kehilangan data
/// (perilaku yang sama dengan `NSAlert` dan `AlertDialog` shadcn).
pub fn alert(fonts: &Fonts, theme: &Theme, title: impl Into<String>) -> DialogBuilder {
    dialog(fonts, theme, title).dismiss(Dismiss::ESCAPE)
}

/// Builder dialog.
///
/// Menjadi [`OverlayBuilder`] saat dimasukkan ke
/// [`crate::overlay_layer`], jadi dialog menumpang infrastruktur overlay yang
/// sama dengan popover/tooltip/menu/toast — tidak ada geometri, dismiss, atau
/// transisi yang dihitung ulang di sini.
pub struct DialogBuilder {
    fonts: Fonts,
    theme: Theme,
    key: Option<Key>,
    title: String,
    message: Option<String>,
    content: Option<View>,
    actions: Vec<DialogAction>,
    order: ButtonOrder,
    open: bool,
    width: f32,
    dismiss: Dismiss,
    on_dismiss: Option<Callback>,
    spring: Spring,
}

impl DialogBuilder {
    /// Isi tambahan di antara pesan dan barisan tombol — form, daftar pilihan,
    /// atau apa pun.
    ///
    /// Di sinilah aturan Return jadi terasa: selama fokus berada di sebuah
    /// kontrol yang **tidak** menelan Return (kolom teks satu baris, misalnya),
    /// Return tetap menjalankan tombol default dialog.
    pub fn content(mut self, content: impl Into<View>) -> Self {
        self.content = Some(content.into());
        self
    }

    /// Kunci identitas — wajib bila dialognya datang dari daftar dinamis (§2.5).
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Terbuka atau tertutup. Perubahannya **memicu transisi**, bukan lompatan.
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// Teks penjelas di bawah judul.
    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Tambahkan satu tombol.
    pub fn action(mut self, action: DialogAction) -> Self {
        self.actions.push(action);
        self
    }

    /// Tambahkan beberapa tombol sekaligus.
    pub fn actions(mut self, actions: impl IntoIterator<Item = DialogAction>) -> Self {
        self.actions.extend(actions);
        self
    }

    /// Tambahkan tombol default (dijalankan Return).
    pub fn confirm(self, label: impl Into<String>, f: impl Fn() + 'static) -> Self {
        self.action(action(label).confirm().on_press(f))
    }

    /// Tambahkan tombol batal (dijalankan Esc, dan klik di luar bila diizinkan).
    pub fn cancel(self, label: impl Into<String>, f: impl Fn() + 'static) -> Self {
        self.action(action(label).cancel().on_press(f))
    }

    /// Tambahkan tombol merusak — **tidak** menjadi tombol default (HIG).
    pub fn destructive(self, label: impl Into<String>, f: impl Fn() + 'static) -> Self {
        self.action(action(label).destructive().on_press(f))
    }

    /// Paksa susunan tombol alih-alih mengikuti konvensi OS.
    ///
    /// Untuk gallery dan uji lintas-platform; aplikasi biasa tidak memakainya.
    pub fn order(mut self, order: ButtonOrder) -> Self {
        self.order = order;
        self
    }

    /// Lebar panel dalam poin logis — **selalu** turunan skala spacing (§2.6).
    ///
    /// Nilainya tetap dibatasi ruang yang tersedia: di window sempit panel ikut
    /// menyempit, tidak pernah menonjol keluar layar.
    pub fn width(mut self, width: f32) -> Self {
        self.width = width.max(0.0);
        self
    }

    /// Cara-cara yang diizinkan untuk menutup.
    pub fn dismiss(mut self, dismiss: Dismiss) -> Self {
        self.dismiss = dismiss;
        self
    }

    /// Apa yang dijalankan saat dialog ditutup pengguna (Esc/klik luar).
    ///
    /// Tanpa ini, yang dijalankan adalah aksi [`ActionKind::Cancel`] — sehingga
    /// "Esc = Batal" benar dengan sendirinya dan tidak perlu ditulis dua kali.
    pub fn on_dismiss(mut self, f: impl Fn() + 'static) -> Self {
        self.on_dismiss = Some(Callback::new(f));
        self
    }

    /// Spring yang menjalankan transisinya (`smooth`/`snappy`/`bouncy`).
    pub fn spring(mut self, spring: Spring) -> Self {
        self.spring = spring;
        self
    }

    /// Tombol-tombol dalam urutan visual yang berlaku.
    pub fn arranged(&self) -> Vec<DialogAction> {
        self.order.arrange(self.actions.clone())
    }

    /// Aksi yang dijalankan Return, bila ada.
    fn default_action(&self) -> Option<Callback> {
        self.actions
            .iter()
            .find(|a| a.kind == ActionKind::Confirm)
            .and_then(DialogAction::callback)
    }

    /// Aksi yang dijalankan Esc/klik luar.
    fn dismiss_action(&self) -> Option<Callback> {
        self.on_dismiss.clone().or_else(|| {
            self.actions
                .iter()
                .find(|a| a.kind == ActionKind::Cancel)
                .and_then(DialogAction::callback)
        })
    }

    /// Panel: judul, pesan, isi tambahan, lalu barisan tombol.
    fn panel(&mut self) -> View {
        let t = &self.theme;
        let mut isi: Vec<View> = vec![self.header()];
        if let Some(konten) = self.content.take() {
            isi.push(konten);
        }
        if !self.actions.is_empty() {
            isi.push(self.tombol());
        }

        let kartu = column(isi)
            .spacing(t.space(5.0))
            .cross(CrossAlign::Stretch)
            .padding(Insets::all(t.space(5.0)))
            .background(t.color.surface_elevated)
            .corners(t.corners(t.radius.xl))
            // Hairline mengikuti skala spacing (0.25 langkah = 1pt): di mode
            // gelap inilah yang memisahkan panel dari peredup di belakangnya.
            .border(t.space(0.25), t.color.separator)
            .shadow(t.shadow.xl);

        // Lebar dijepit ke ruang yang tersedia oleh `BoxConstraints::enforce`,
        // jadi window yang lebih sempit dari dialognya tetap benar.
        let kotak = constrained(
            BoxConstraints::new(self.width, self.width, 0.0, f32::INFINITY),
            kartu,
        );

        Builder::new(DialogPanelProps {
            open: self.open,
            default_action: self.default_action(),
        })
        .child(kotak)
        .into()
    }

    /// Judul + pesan.
    fn header(&self) -> View {
        let t = &self.theme;
        let mut baris: Vec<View> = Vec::with_capacity(2);
        baris.push(
            gaya(text(&self.fonts, &self.title), t.typography.headline)
                .weight(FontWeight::SEMIBOLD)
                .color(t.color.label)
                // Judul dibacakan sekali, dari node dialognya — bukan dua kali.
                .role(AccessRole::Container)
                .into(),
        );
        if let Some(pesan) = &self.message {
            baris.push(
                gaya(text(&self.fonts, pesan), t.typography.body)
                    .color(t.color.secondary_label)
                    .into(),
            );
        }
        column(baris)
            .spacing(t.space(2.0))
            .cross(CrossAlign::Stretch)
            .into()
    }

    /// Barisan tombol dalam urutan visual platform.
    fn tombol(&self) -> View {
        let t = &self.theme;
        let tombol: Vec<View> = self
            .arranged()
            .into_iter()
            .map(|a| {
                let mut b = button_variant(&self.fonts, t, a.label(), a.variant())
                    .disabled(a.is_disabled());
                if let Some(cb) = a.callback() {
                    b = b.on_press(move || cb.call());
                }
                b.into()
            })
            .collect();
        row(tombol)
            // Tombol dialog rata ke akhir baris di ketiga OS; yang berbeda cuma
            // urutannya (`ButtonOrder`). Di RTL barisnya tercermin sendiri.
            .main(MainAlign::End)
            .cross(CrossAlign::Center)
            .spacing(t.space(3.0))
            .wrap()
            .into()
    }
}

/// Terapkan sebuah token tipografi ke teks.
fn gaya(teks: Text, style: TypeStyle) -> Text {
    teks.size(style.size)
        .line_height(style.line_height)
        .tracking(style.tracking)
        .weight(FontWeight(style.weight))
}

impl From<DialogBuilder> for OverlayBuilder {
    fn from(mut b: DialogBuilder) -> OverlayBuilder {
        let t = b.theme;
        let mut ov = overlay(b.panel())
            .open(b.open)
            .barrier(Barrier::Modal)
            .backdrop(t.color.scrim)
            .placement(Placement::center())
            .dismiss(b.dismiss)
            .role(AccessRole::Dialog)
            .label(b.title.clone())
            .spring(b.spring);
        if let Some(cb) = b.dismiss_action() {
            ov = ov.on_dismiss(move || cb.call());
        }
        if let Some(key) = b.key.clone() {
            ov = ov.key(key);
        }
        ov
    }
}

impl From<DialogBuilder> for View {
    fn from(b: DialogBuilder) -> View {
        View::from(OverlayBuilder::from(b))
    }
}

impl core::fmt::Debug for DialogBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DialogBuilder")
            .field("title", &self.title)
            .field("open", &self.open)
            .field("actions", &self.actions.len())
            .field("order", &self.order.resolved())
            .finish()
    }
}

#[cfg(test)]
mod tests;
