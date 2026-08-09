//! **Signals + rebuild per-komponen** — model state framework (REKOMENDASI §2.5).
//!
//! Keputusan yang mengikat: state lokal komponen memakai [`use_signal`]; setiap
//! pembacaan signal **selama build** mendaftarkan komponen itu sebagai pembaca;
//! setiap tulisan menandai para pembacanya *dirty* → subtree kecil itu dibangun
//! ulang → di-diff. Ini pola Dioxus 0.7, dan mental model-nya paling dekat
//! dengan `setState` Flutter.
//!
//! Harga yang diterima sadar (dan disediakan di sini): **scheduler
//! dirty-marking + scope tracking** di internal framework, dan **disiplin
//! key/identity di list dinamis**.
//!
//! ```
//! use silka_core::signals::{use_signal, Runtime};
//!
//! let rt = Runtime::new();
//! let count = rt.signal(0i32);
//!
//! // Komponen membaca signal saat dibangun → ia berlangganan.
//! rt.build_root(|| {
//!     let _teks = format!("Nilai: {}", count.get());
//! });
//! assert!(!rt.is_dirty(rt.root()));
//!
//! // Tulisan dari event handler menandai pembacanya dirty.
//! count.set(1);
//! assert_eq!(rt.drain_dirty(), vec![rt.root()]);
//! ```
//!
//! ## Aturan main
//!
//! - **Membaca melacak, menulis menandai.** [`Signal::get`]/[`Signal::with`]
//!   berlangganan bila dipanggil saat build; di luar build (event handler,
//!   hasil async) mereka hanya membaca. [`Signal::peek`] tidak pernah
//!   berlangganan.
//! - **Langganan dibangun ulang tiap build.** Komponen yang berhenti membaca
//!   sebuah signal berhenti dibangunkan olehnya — tidak ada langganan basi.
//! - **Hook tidak boleh kondisional.** `use_signal` dicocokkan per urutan
//!   pemanggilan; berubah urutan/jumlahnya = panik dengan pesan jelas, bukan
//!   state yang tertukar diam-diam.
//! - **Anak wajib punya kunci.** [`scope`] dan [`list`] memakai [`Key`] sebagai
//!   identitas; kunci yang sama = state yang sama walau posisinya bergeser.
//! - **Batching itu tentang bangun-tidurnya renderer**, bukan tentang nilai:
//!   nilai berubah seketika, [`Runtime::batch`] hanya menyatukan pemberitahuan
//!   ke scheduler menjadi satu.
//!
//! ## Sambungan ke scheduler
//!
//! [`Runtime::on_wake`] dipanggil sekali per flush dengan [`SIGNAL_DIRTY`].
//! Sambungkan ke [`crate::scheduler::FrameScheduler::request`] dan render tetap
//! **hanya saat dirty** (§3.5) — signal yang tidak dibaca siapa pun tidak
//! membangunkan GPU sama sekali.

mod runtime;
#[cfg(test)]
mod tests;

use std::fmt;
use std::marker::PhantomData;

use runtime::{current_build, run_untracked, runtime_of};
pub use runtime::{Runtime, RuntimeId, ScopeId, SignalId, SIGNAL_DIRTY};

// ---------------------------------------------------------------------------
// Key
// ---------------------------------------------------------------------------

/// Identitas sebuah scope di antara saudara-saudaranya.
///
/// Inilah "disiplin key" §2.5: pada list dinamis, kunci — bukan posisi — yang
/// menentukan state milik siapa. Menggeser, menyisipkan, atau menukar item
/// tidak memindahkan state selama kuncinya ikut.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    /// Kunci scope akar; tidak pernah dibuat pengguna.
    Root,
    /// Kunci numerik (id baris database, indeks, enum diskriminan).
    Num(i64),
    /// Kunci teks (uuid, nama slot, path).
    Text(Box<str>),
}

impl Key {
    /// Kunci numerik.
    pub fn num(n: impl Into<i64>) -> Self {
        Key::Num(n.into())
    }

    /// Kunci teks.
    pub fn text(s: impl AsRef<str>) -> Self {
        Key::Text(s.as_ref().into())
    }
}

impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Root => f.write_str("Key(root)"),
            Key::Num(n) => write!(f, "Key({n})"),
            Key::Text(s) => write!(f, "Key({s:?})"),
        }
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Root => f.write_str("root"),
            Key::Num(n) => write!(f, "{n}"),
            Key::Text(s) => f.write_str(s),
        }
    }
}

macro_rules! key_from_num {
    ($($t:ty),*) => {$(
        impl From<$t> for Key {
            fn from(v: $t) -> Self {
                Key::Num(v as i64)
            }
        }
    )*};
}
key_from_num!(i8, i16, i32, i64, u8, u16, u32, usize);

impl From<&str> for Key {
    fn from(v: &str) -> Self {
        Key::Text(v.into())
    }
}

impl From<String> for Key {
    fn from(v: String) -> Self {
        Key::Text(v.into_boxed_str())
    }
}

impl From<&String> for Key {
    fn from(v: &String) -> Self {
        Key::Text(v.as_str().into())
    }
}

// ---------------------------------------------------------------------------
// Signal
// ---------------------------------------------------------------------------

/// Nilai reaktif milik runtime.
///
/// `Signal` hanyalah ID — `Copy`, seukuran tiga `u32`, dan boleh masuk ke
/// closure `move` sebanyak yang diperlukan. Itulah yang membuat gaya penulisan
/// §2.5 mungkin:
///
/// ```ignore
/// let count = use_signal(|| 0);
/// column((
///     text(format!("Nilai: {}", count.get())),
///     button("Tambah").on_press(move || count.set(count.get() + 1)),
/// ))
/// ```
///
/// Signal terikat ke thread runtime-nya (UI thread) dan sengaja **bukan**
/// `Send`: hasil async kembali lewat scheduler, bukan lewat signal lintas
/// thread.
pub struct Signal<T: 'static> {
    id: SignalId,
    marker: PhantomData<*const T>,
}

impl<T: 'static> Signal<T> {
    pub(crate) fn from_id(id: SignalId) -> Self {
        Self {
            id,
            marker: PhantomData,
        }
    }

    /// Identitas signal ini.
    pub fn id(&self) -> SignalId {
        self.id
    }

    /// Benar bila signal masih hidup (scope pemiliknya belum dibuang).
    pub fn is_alive(&self) -> bool {
        runtime_of(self.id).is_signal_alive(self.id)
    }

    /// Baca lewat referensi — **melacak** bila dipanggil saat build.
    ///
    /// Closure tidak boleh membaca atau menulis signal yang sama (akses
    /// rekursif dilaporkan sebagai panik yang jelas).
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let rt = runtime_of(self.id);
        rt.track(self.id);
        rt.with_value(self.id, f)
    }

    /// Baca salinan nilainya — **melacak** bila dipanggil saat build.
    pub fn get(&self) -> T
    where
        T: Clone,
    {
        self.with(|v| v.clone())
    }

    /// Baca lewat referensi **tanpa** melacak.
    pub fn peek_with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        runtime_of(self.id).with_value(self.id, f)
    }

    /// Baca salinan nilainya **tanpa** melacak.
    pub fn peek(&self) -> T
    where
        T: Clone,
    {
        self.peek_with(|v| v.clone())
    }

    /// Tulis nilai baru dan tandai seluruh pembacanya dirty.
    pub fn set(&self, value: T) {
        let _ = self.replace(value);
    }

    /// Tulis nilai baru dan kembalikan yang lama.
    pub fn replace(&self, value: T) -> T {
        runtime_of(self.id).replace_value(self.id, value)
    }

    /// Ubah di tempat; **selalu** menandai dirty (runtime tidak bisa tahu
    /// apakah closure benar-benar mengubah sesuatu).
    pub fn update<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        runtime_of(self.id).update_value(self.id, f)
    }

    /// Tulis hanya bila nilainya benar-benar berbeda.
    ///
    /// Mengembalikan `true` bila ada perubahan (dan renderer dibangunkan).
    /// Inilah bentuk yang dipakai saat sumbernya berisik — mis. hasil polling
    /// yang sering mengirim nilai yang sama.
    pub fn set_if_changed(&self, value: T) -> bool
    where
        T: PartialEq,
    {
        if self.peek_with(|cur| *cur == value) {
            return false;
        }
        self.set(value);
        true
    }
}

impl<T: 'static> Clone for Signal<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for Signal<T> {}

impl<T: 'static> PartialEq for Signal<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl<T: 'static> Eq for Signal<T> {}

impl<T: 'static> std::hash::Hash for Signal<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

impl<T: 'static> fmt::Debug for Signal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.id)
    }
}

// ---------------------------------------------------------------------------
// Hook & scope
// ---------------------------------------------------------------------------

/// State lokal komponen (§2.5) — dibuat sekali, bertahan lintas rebuild.
///
/// `init` hanya dijalankan pada build pertama scope ini. Pada build berikutnya
/// hook yang sama dikenali dari **urutan pemanggilannya**, jadi `use_signal`
/// tidak boleh berada di dalam `if`/`loop` — pelanggaran dilaporkan sebagai
/// panik, bukan state yang tertukar.
///
/// Panik bila dipanggil di luar build komponen.
pub fn use_signal<T: 'static>(init: impl FnOnce() -> T) -> Signal<T> {
    let (rt_id, scope) = current_build()
        .expect("use_signal hanya boleh dipanggil saat komponen dibangun (di dalam build_root/scope/rebuild)");
    let rt = Runtime::current().expect("runtime yang sedang membangun harus hidup");
    debug_assert_eq!(rt.id(), rt_id);
    Signal::from_id(rt.use_signal_hook::<T>(scope, init))
}

/// Bangun satu komponen anak dengan identitas `key`.
///
/// Kunci yang sama pada build berikutnya = scope yang sama = state yang sama,
/// walau urutannya berubah. Kunci yang hilang = scope-nya dibuang beserta
/// seluruh subtree, hook, dan langganannya.
///
/// Panik bila dipanggil di luar build, atau bila `key` sudah dipakai saudara
/// lain pada build yang sama.
pub fn scope<R>(key: impl Into<Key>, body: impl FnOnce() -> R) -> R {
    let (_, parent) =
        current_build().expect("scope() hanya boleh dipanggil saat komponen dibangun");
    let rt = Runtime::current().expect("runtime yang sedang membangun harus hidup");
    let child = rt.reconcile_child(parent, key.into());
    rt.run_scope(child, body)
        .expect("scope anak baru saja dibuat, tidak mungkin mati")
}

/// Bangun satu komponen anak per item, dengan kunci dari `key`.
///
/// Bentuk ringkas dari [`scope`] untuk list dinamis. Menukar urutan item
/// memindahkan scope-nya, bukan state-nya.
///
/// ```
/// use silka_core::signals::{list, use_signal, Key, Runtime};
///
/// let rt = Runtime::new();
/// let mut baris: Vec<i64> = vec![1, 2, 3];
///
/// rt.build_root(|| {
///     list(baris.iter().copied(), |id| Key::num(*id), |id| use_signal(|| *id))
/// });
/// let awal = rt.children(rt.root());
///
/// // Item ditukar urutannya: scope-nya ikut pindah, identitasnya tetap.
/// baris.swap(0, 2);
/// rt.build_root(|| {
///     list(baris.iter().copied(), |id| Key::num(*id), |id| use_signal(|| *id))
/// });
/// assert_eq!(rt.children(rt.root()), vec![awal[2], awal[1], awal[0]]);
/// ```
pub fn list<I, K, F, R>(items: I, key: K, mut body: F) -> Vec<R>
where
    I: IntoIterator,
    K: Fn(&I::Item) -> Key,
    F: FnMut(&I::Item) -> R,
{
    items
        .into_iter()
        .map(|item| {
            let k = key(&item);
            scope(k, || body(&item))
        })
        .collect()
}

/// Jalankan `f` tanpa melacak pembacaan signal apa pun.
///
/// Dipakai saat komponen perlu *melihat* nilai tanpa ingin dibangun ulang
/// ketika nilai itu berubah.
pub fn untracked<R>(f: impl FnOnce() -> R) -> R {
    run_untracked(f)
}

/// Scope yang sedang dibangun di thread ini, bila ada.
pub fn current_scope() -> Option<ScopeId> {
    current_build().map(|(_, scope)| scope)
}
