//! [`Callback`] — an action the application hands to an interactive node.
//!
//! This is the shape of `on_press` promised in REKOMENDASI §2.5:
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
//! Three properties make it suitable for a render node to hold:
//!
//! 1. **Cheap to `Clone`** ([`std::rc::Rc`]) — a node may copy it out before
//!    invoking it, so the handler never runs while the node itself is borrowed
//!    `&mut`. That matters: handlers usually write to signals, and a signal
//!    write schedules a frame through that very same path.
//! 2. **Identity-based `PartialEq`** — two closures rebuilt on every rebuild are
//!    genuinely **not** equal, and that is correct: the newest props always
//!    replace the old ones, without comparing contents that cannot be compared.
//! 3. **Never touches the tree.** All a callback may do is write to a signal;
//!    structural change is the view-diff's business (§2.5).

use std::fmt;
use std::rc::Rc;

/// A zero-argument action the application hands to a node.
#[derive(Clone)]
pub struct Callback(Rc<dyn Fn()>);

impl Callback {
    /// Wrap a closure.
    pub fn new(f: impl Fn() + 'static) -> Self {
        Self(Rc::new(f))
    }

    /// Run the action.
    pub fn call(&self) {
        (self.0)()
    }
}

impl PartialEq for Callback {
    /// Identity, not contents: the same `Rc` means the same callback.
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
