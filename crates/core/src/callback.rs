//! [`Callback`] — aksi yang dititipkan aplikasi ke sebuah node interaktif.
//!
//! Inilah bentuk `on_press` yang dijanjikan REKOMENDASI §2.5:
//!
//! ```
//! # use silka_core::signals::Runtime;
//! # let rt = Runtime::new();
//! # let count = rt.signal(0i32);
//! use silka_core::Callback;
//!
//! let naikkan = Callback::new(move || count.set(count.get() + 1));
//! naikkan.call();
//! assert_eq!(count.get(), 1);
//! ```
//!
//! Tiga sifat yang membuatnya cocok dipegang node render:
//!
//! 1. **`Clone` murah** ([`std::rc::Rc`]) — node boleh menyalinnya keluar
//!    sebelum memanggilnya, sehingga handler tidak berjalan sambil node-nya
//!    dipinjam `&mut`. Itu penting: handler biasanya menulis signal, dan
//!    tulisan signal menjadwalkan frame lewat jalur yang sama.
//! 2. **`PartialEq` berdasarkan identitas** — dua closure yang dibangun ulang
//!    setiap rebuild memang **tidak** sama, dan itu benar: props terbaru selalu
//!    menggantikan yang lama, tanpa membandingkan isi yang tidak bisa
//!    dibandingkan.
//! 3. **Tidak pernah menyentuh pohon.** Yang boleh dilakukan sebuah callback
//!    adalah menulis signal; perubahan struktur adalah wewenang view-diff
//!    (§2.5).

use std::fmt;
use std::rc::Rc;

/// Aksi tanpa argumen yang dititipkan aplikasi ke sebuah node.
#[derive(Clone)]
pub struct Callback(Rc<dyn Fn()>);

impl Callback {
    /// Bungkus sebuah closure.
    pub fn new(f: impl Fn() + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Jalankan aksinya.
    pub fn call(&self) {
        (self.0)()
    }
}

impl PartialEq for Callback {
    /// Identitas, bukan isi: dua `Rc` yang sama = callback yang sama.
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Debug for Callback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Callback")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn memanggil_closure_yang_dibungkus() {
        let n = Rc::new(Cell::new(0));
        let cb = {
            let n = n.clone();
            Callback::new(move || n.set(n.get() + 1))
        };
        cb.call();
        cb.clone().call();
        assert_eq!(n.get(), 2);
    }

    #[test]
    fn kesamaan_adalah_identitas() {
        let a = Callback::new(|| {});
        let b = Callback::new(|| {});
        assert_eq!(a, a.clone());
        assert_ne!(a, b, "dua closure berbeda tidak pernah dianggap sama");
    }
}
