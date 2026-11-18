//! A small, byte-bounded least-recently-used cache.
use std::collections::VecDeque;

pub struct MemoryCache<T> {
    entries: VecDeque<(String, T, usize)>,
    bytes: usize,
    max_bytes: usize,
    max_entries: usize,
}

impl<T: Clone> MemoryCache<T> {
    pub fn new(max_bytes: usize, max_entries: usize) -> Self {
        Self { entries: VecDeque::new(), bytes: 0, max_bytes, max_entries }
    }

    pub fn get(&mut self, key: &str) -> Option<T> {
        let pos = self.entries.iter().position(|entry| entry.0 == key)?;
        let entry = self.entries.remove(pos)?;
        let value = entry.1.clone();
        self.entries.push_back(entry);
        Some(value)
    }

    pub fn insert(&mut self, key: String, value: T, bytes: usize) -> Vec<T> {
        let mut evicted = Vec::new();
        if let Some(pos) = self.entries.iter().position(|entry| entry.0 == key) {
            let (_, old, size) = self.entries.remove(pos).unwrap();
            self.bytes -= size;
            evicted.push(old);
        }
        self.entries.push_back((key, value, bytes));
        self.bytes += bytes;
        // Keep one oversized image so it isn't decoded again on every frame.
        while self.entries.len() > 1 && (self.bytes > self.max_bytes || self.entries.len() > self.max_entries) {
            let (_, old, size) = self.entries.pop_front().unwrap();
            self.bytes -= size;
            evicted.push(old);
        }
        evicted
    }
}

#[cfg(test)]
mod tests {
    use super::MemoryCache;

    #[test]
    fn eviction_respects_bytes_and_recent_access() {
        let mut cache = MemoryCache::new(10, 3);
        cache.insert("a".into(), 1, 4);
        cache.insert("b".into(), 2, 4);
        assert_eq!(cache.get("a"), Some(1));
        assert_eq!(cache.insert("c".into(), 3, 4), vec![2]);
        assert_eq!(cache.get("b"), None);
        assert_eq!(cache.get("a"), Some(1));
        assert_eq!(cache.insert("big".into(), 4, 20).len(), 2);
        assert_eq!(cache.get("big"), Some(4));
    }
}
