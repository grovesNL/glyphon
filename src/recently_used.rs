use std::{
    borrow::Borrow,
    collections::{hash_map::Entry, HashMap},
    hash::Hash,
};

struct RecentlyUsedItem<V> {
    entry_idx: usize,
    value: V,
}

pub struct RecentlyUsedMap<K: Clone + Copy + Eq + Hash, V> {
    keys: Vec<K>,
    map: HashMap<K, RecentlyUsedItem<V>>,
}

impl<K: Clone + Copy + Eq + Hash, V> RecentlyUsedMap<K, V> {
    pub fn new() -> Self {
        Self::with_capacity(0)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            keys: Vec::with_capacity(capacity),
            map: HashMap::with_capacity(capacity),
        }
    }

    pub fn insert(&mut self, key: K, value: V) {
        let new_idx = self.keys.len();
        match self.map.entry(key) {
            Entry::Occupied(mut occupied) => {
                let old = occupied.insert(RecentlyUsedItem {
                    entry_idx: new_idx,
                    value,
                });
                let removed = self.keys.remove(old.entry_idx);
                self.keys.push(removed);
            }
            Entry::Vacant(vacant) => {
                vacant.insert(RecentlyUsedItem {
                    entry_idx: new_idx,
                    value,
                });
                self.keys.push(key);
            }
        }
    }

    pub fn remove<Q: ?Sized>(&mut self, key: &Q)
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        if let Some(entry) = self.map.remove(key.borrow()) {
            self.keys.remove(entry.entry_idx);
        }
    }

    pub fn get<Q: ?Sized>(&self, k: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.map.get(k).map(|item| &item.value)
    }

    pub fn contains_key<Q: ?Sized>(&self, k: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.map.contains_key(k)
    }

    pub fn entries_least_recently_used(&self) -> impl Iterator<Item = (K, &V)> + '_ {
        self.keys.iter().map(|k| (*k, &self.map[k].value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert() {
        let mut rus = RecentlyUsedMap::new();
        rus.insert("a", ());
        rus.insert("b", ());
        assert_eq!(
            rus.entries_least_recently_used()
                .map(|(k, _)| k)
                .collect::<String>(),
            "ab"
        );
    }

    #[test]
    fn reinsert() {
        let mut rus = RecentlyUsedMap::new();
        rus.insert("a", ());
        rus.insert("b", ());
        rus.insert("c", ());
        rus.insert("a", ());
        assert_eq!(
            rus.entries_least_recently_used()
                .map(|(k, _)| k)
                .collect::<String>(),
            "bca"
        );
    }

    #[test]
    fn remove() {
        let mut rus = RecentlyUsedMap::new();
        rus.insert("a", ());
        rus.insert("b", ());
        rus.remove("a");
        rus.remove("c");
        assert_eq!(
            rus.entries_least_recently_used()
                .map(|(k, _)| k)
                .collect::<String>(),
            "b"
        );
    }
}
