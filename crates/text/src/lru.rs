//! A bounded map that throws away the **least recently used** entry.
//!
//! The measure cache used to react to a full cache by emptying it. That is the
//! worst possible policy for the one moment it matters: while a window is being
//! resized, a burst of new entries arrives every frame, so the cache would be
//! wiped exactly when the previous frame's work was about to pay off. Dropping
//! one cold entry instead keeps the warm ones — the labels, headers and file
//! names that never change — alive across the whole gesture.
//!
//! The structure is the textbook one: a hash index into a slab of slots that
//! are threaded on an intrusive doubly linked list, most recently used first.
//! Lookup, insert and eviction are all O(1); no entry is ever moved in memory,
//! so the links stay valid.

use std::collections::HashMap;
use std::hash::Hash;

/// The end of the list — a slot index that cannot exist.
const NIL: usize = usize::MAX;

struct Slot<K, V> {
    key: K,
    value: V,
    /// Towards the most recently used end.
    prev: usize,
    /// Towards the least recently used end.
    next: usize,
}

/// A fixed-capacity map with least-recently-used eviction.
pub(crate) struct LruMap<K, V> {
    index: HashMap<K, usize>,
    slots: Vec<Option<Slot<K, V>>>,
    free: Vec<usize>,
    /// Most recently used.
    head: usize,
    /// Least recently used — the next victim.
    tail: usize,
    capacity: usize,
}

impl<K: Eq + Hash + Clone, V> LruMap<K, V> {
    /// An empty map that will never hold more than `capacity` entries.
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            index: HashMap::new(),
            slots: Vec::new(),
            free: Vec::new(),
            head: NIL,
            tail: NIL,
            capacity: capacity.max(1),
        }
    }

    /// How many entries are live.
    pub(crate) fn len(&self) -> usize {
        self.index.len()
    }

    /// The upper bound on [`LruMap::len`].
    #[cfg(test)]
    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }

    /// Look an entry up **and mark it as freshly used**.
    pub(crate) fn get(&mut self, key: &K) -> Option<&V> {
        let slot = *self.index.get(key)?;
        self.ke_depan(slot);
        Some(&self.slot(slot).value)
    }

    /// Look an entry up without changing its position — for tests and
    /// diagnostics, never on a hot path.
    #[cfg(test)]
    pub(crate) fn peek(&self, key: &K) -> Option<&V> {
        let slot = *self.index.get(key)?;
        Some(&self.slot(slot).value)
    }

    /// Insert (or overwrite) an entry, evicting the least recently used one
    /// when the map is full.
    pub(crate) fn insert(&mut self, key: K, value: V) {
        if let Some(&slot) = self.index.get(&key) {
            self.slot_mut(slot).value = value;
            self.ke_depan(slot);
            return;
        }
        if self.index.len() >= self.capacity {
            self.buang_terlama();
        }
        let baru = Slot {
            key: key.clone(),
            value,
            prev: NIL,
            next: NIL,
        };
        let slot = match self.free.pop() {
            Some(s) => {
                self.slots[s] = Some(baru);
                s
            }
            None => {
                self.slots.push(Some(baru));
                self.slots.len() - 1
            }
        };
        self.index.insert(key, slot);
        self.sambung_di_depan(slot);
    }

    /// Drop every entry.
    pub(crate) fn clear(&mut self) {
        self.index.clear();
        self.slots.clear();
        self.free.clear();
        self.head = NIL;
        self.tail = NIL;
    }

    /// Keep only the entries the predicate accepts.
    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&K, &V) -> bool) {
        let mut buang = Vec::new();
        for slot in self.slots.iter().flatten() {
            if !keep(&slot.key, &slot.value) {
                buang.push(slot.key.clone());
            }
        }
        for key in buang {
            self.remove(&key);
        }
    }

    /// Remove one entry, if it is there.
    fn remove(&mut self, key: &K) {
        let Some(slot) = self.index.remove(key) else {
            return;
        };
        self.lepas(slot);
        self.slots[slot] = None;
        self.free.push(slot);
    }

    fn buang_terlama(&mut self) {
        if self.tail == NIL {
            return;
        }
        let korban = self.tail;
        let key = self.slot(korban).key.clone();
        self.index.remove(&key);
        self.lepas(korban);
        self.slots[korban] = None;
        self.free.push(korban);
    }

    fn slot(&self, i: usize) -> &Slot<K, V> {
        self.slots[i].as_ref().expect("live slot")
    }

    fn slot_mut(&mut self, i: usize) -> &mut Slot<K, V> {
        self.slots[i].as_mut().expect("live slot")
    }

    /// Unlink a slot from the recency list, leaving the slot itself alone.
    fn lepas(&mut self, slot: usize) {
        let (prev, next) = {
            let s = self.slot(slot);
            (s.prev, s.next)
        };
        if prev != NIL {
            self.slot_mut(prev).next = next;
        } else {
            self.head = next;
        }
        if next != NIL {
            self.slot_mut(next).prev = prev;
        } else {
            self.tail = prev;
        }
        let s = self.slot_mut(slot);
        s.prev = NIL;
        s.next = NIL;
    }

    fn sambung_di_depan(&mut self, slot: usize) {
        let lama = self.head;
        {
            let s = self.slot_mut(slot);
            s.prev = NIL;
            s.next = lama;
        }
        if lama != NIL {
            self.slot_mut(lama).prev = slot;
        } else {
            self.tail = slot;
        }
        self.head = slot;
    }

    fn ke_depan(&mut self, slot: usize) {
        if self.head == slot {
            return;
        }
        self.lepas(slot);
        self.sambung_di_depan(slot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menyimpan_dan_mengambil_kembali() {
        let mut lru: LruMap<&str, u32> = LruMap::new(4);
        lru.insert("a", 1);
        lru.insert("b", 2);
        assert_eq!(lru.len(), 2);
        assert_eq!(lru.get(&"a"), Some(&1));
        assert_eq!(lru.get(&"b"), Some(&2));
        assert_eq!(lru.get(&"c"), None);
    }

    #[test]
    fn menulis_ulang_kunci_yang_sama_bukan_entri_baru() {
        let mut lru: LruMap<&str, u32> = LruMap::new(4);
        lru.insert("a", 1);
        lru.insert("a", 9);
        assert_eq!(lru.len(), 1);
        assert_eq!(lru.get(&"a"), Some(&9));
    }

    #[test]
    fn penuh_membuang_yang_paling_lama_bukan_semuanya() {
        let mut lru: LruMap<u32, u32> = LruMap::new(3);
        for i in 0..3 {
            lru.insert(i, i);
        }
        lru.insert(3, 3);
        // Only the oldest entry left; this is the whole point of the change.
        assert_eq!(lru.len(), 3);
        assert_eq!(lru.peek(&0), None);
        assert_eq!(lru.peek(&1), Some(&1));
        assert_eq!(lru.peek(&3), Some(&3));
    }

    #[test]
    fn entri_yang_dipakai_bertahan_dari_gelombang_entri_baru() {
        let mut lru: LruMap<u32, u32> = LruMap::new(8);
        lru.insert(1000, 1);
        // A resize-like flood of one-off keys, with the warm entry touched
        // every few inserts — exactly the pattern a window resize produces.
        for i in 0..100 {
            lru.insert(i, i);
            assert_eq!(lru.get(&1000), Some(&1), "entri panas hilang di i={i}");
        }
        assert_eq!(lru.len(), 8);
    }

    #[test]
    fn kapasitas_tidak_pernah_terlampaui() {
        let mut lru: LruMap<u32, u32> = LruMap::new(5);
        for i in 0..1000 {
            lru.insert(i, i);
            assert!(lru.len() <= lru.capacity());
        }
        assert_eq!(lru.len(), 5);
        // The five most recent survived.
        for i in 995..1000 {
            assert_eq!(lru.peek(&i), Some(&i));
        }
    }

    #[test]
    fn retain_menyaring_dan_membebaskan_slot() {
        let mut lru: LruMap<u32, u32> = LruMap::new(16);
        for i in 0..10 {
            lru.insert(i, i);
        }
        lru.retain(|k, _| k % 2 == 0);
        assert_eq!(lru.len(), 5);
        assert_eq!(lru.peek(&3), None);
        assert_eq!(lru.peek(&4), Some(&4));
        // The freed slots are reused rather than leaked.
        for i in 100..110 {
            lru.insert(i, i);
        }
        assert_eq!(lru.len(), 15);
        assert_eq!(lru.peek(&4), Some(&4));
    }

    #[test]
    fn clear_mengosongkan_semuanya() {
        let mut lru: LruMap<u32, u32> = LruMap::new(4);
        lru.insert(1, 1);
        lru.insert(2, 2);
        lru.clear();
        assert_eq!(lru.len(), 0);
        assert_eq!(lru.get(&1), None);
        lru.insert(3, 3);
        assert_eq!(lru.get(&3), Some(&3));
    }

    #[test]
    fn urutan_pemakaian_menentukan_korban_berikutnya() {
        let mut lru: LruMap<&str, u32> = LruMap::new(2);
        lru.insert("a", 1);
        lru.insert("b", 2);
        // Touching "a" makes "b" the oldest.
        assert_eq!(lru.get(&"a"), Some(&1));
        lru.insert("c", 3);
        assert_eq!(lru.peek(&"b"), None);
        assert_eq!(lru.peek(&"a"), Some(&1));
    }
}
