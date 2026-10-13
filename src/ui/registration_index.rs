//! Frame-derived ID counts. Generation tags clear the index without freeing
//! table storage, so a warmed frame does not allocate for ID validation.
use alloc::vec::Vec;

#[derive(Clone, Copy, Default)]
struct Entry {
    id: u64,
    count: usize,
    generation: u64,
}
pub(super) struct RegistrationIndex {
    entries: Vec<Entry>,
    generation: u64,
    used: usize,
}
impl Default for RegistrationIndex {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            generation: 1,
            used: 0,
        }
    }
}
impl RegistrationIndex {
    pub fn clear(&mut self) {
        if self.generation == u64::MAX {
            self.entries.fill(Entry::default());
            self.generation = 1;
        } else {
            self.generation += 1;
        }
        self.used = 0;
    }
    fn hash(id: u64) -> usize {
        let mut value = id;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        (value ^ (value >> 31)) as usize
    }
    fn slot(&self, id: u64) -> usize {
        let mask = self.entries.len() - 1;
        let mut slot = Self::hash(id) & mask;
        while self.entries[slot].generation == self.generation && self.entries[slot].id != id {
            slot = (slot + 1) & mask;
        }
        slot
    }
    pub fn contains(&self, id: u64) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        let entry = self.entries[self.slot(id)];
        entry.generation == self.generation && entry.count > 0
    }
    pub fn insert(&mut self, id: u64) {
        if self.entries.is_empty() || self.used >= self.entries.len() / 2 {
            self.grow();
        }
        let slot = self.slot(id);
        let entry = &mut self.entries[slot];
        if entry.generation != self.generation {
            *entry = Entry {
                id,
                count: 0,
                generation: self.generation,
            };
            self.used += 1;
        }
        entry.count += 1;
    }
    pub fn remove(&mut self, id: u64) {
        let slot = self.slot(id);
        let entry = &mut self.entries[slot];
        assert!(entry.generation == self.generation && entry.count > 0);
        // Keep the probe chain intact until the next generation.
        entry.count -= 1;
    }
    fn grow(&mut self) {
        let capacity = self
            .entries
            .len()
            .checked_mul(2)
            .expect("index address space")
            .max(8);
        let previous =
            core::mem::replace(&mut self.entries, alloc::vec![Entry::default(); capacity]);
        self.used = 0;
        for entry in previous {
            if entry.generation == self.generation && entry.count > 0 {
                let slot = self.slot(entry.id);
                self.entries[slot] = entry;
                self.used += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_rollback_collisions_and_frame_reuse() {
        let mut index = RegistrationIndex::default();
        index.clear();
        for id in 0..20_000 {
            index.insert(id << 32);
            index.insert(id << 32);
        }
        for id in 0..20_000 {
            index.remove(id << 32);
            assert!(index.contains(id << 32));
            index.remove(id << 32);
            assert!(!index.contains(id << 32));
        }
        let storage = index.entries.as_ptr();
        index.clear();
        for id in 0..20_000 {
            index.insert(id);
        }
        assert_eq!(storage, index.entries.as_ptr());
        index.generation = u64::MAX;
        index.clear();
        assert!(!index.contains(0));
        index.insert(0);
        assert!(index.contains(0));
    }
}
