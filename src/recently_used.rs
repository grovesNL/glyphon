use std::{
    borrow::Borrow,
    collections::{hash_map::Entry, HashMap},
    hash::Hash,
};

struct RecentlyUsedItem<V> {
    node_idx: usize,
    value: V,
}

enum Node<V> {
    Value {
        value: V,
        prev_idx: Option<usize>,
        next_idx: Option<usize>,
    },
    Free {
        next_idx: Option<usize>,
    },
}

pub struct RecentlyUsedMap<K: Clone + Copy + Eq + Hash, V> {
    nodes: Vec<Node<K>>,
    least_recent_idx: Option<usize>,
    most_recent_idx: Option<usize>,
    map: HashMap<K, RecentlyUsedItem<V>>,
    free: Option<usize>,
}

impl<K: Clone + Copy + Eq + Hash, V> RecentlyUsedMap<K, V> {
    pub fn new() -> Self {
        Self::with_capacity(0)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        assert!(capacity < usize::MAX - 1, "capacity too large");
        Self {
            nodes: Vec::with_capacity(capacity),
            least_recent_idx: None,
            most_recent_idx: None,
            free: None,
            map: HashMap::with_capacity(capacity),
        }
    }

    fn allocate_node(&mut self, node: Node<K>) -> usize {
        match self.free {
            Some(idx) => {
                // Reuse a node from storage
                match self.nodes[idx] {
                    Node::Free { next_idx } => {
                        self.free = next_idx;
                    }
                    Node::Value { .. } => unreachable!(),
                }

                self.nodes[idx] = node;

                idx
            }
            None => {
                // Create a new storage node
                self.nodes.push(node);
                self.nodes.len() - 1
            }
        }
    }

    pub fn insert(&mut self, key: K, value: V) {
        let new_idx = self.allocate_node(Node::Value {
            value: key,
            prev_idx: self.most_recent_idx,
            next_idx: None,
        });

        let prior_most_recent = self.most_recent_idx.map(|idx| &mut self.nodes[idx]);
        match prior_most_recent {
            Some(p) => {
                // Update prior most recent if one exists
                match p {
                    Node::Value { next_idx, .. } => {
                        *next_idx = Some(new_idx);
                    }
                    Node::Free { .. } => unreachable!(),
                }
            }
            None => {
                // Update least recent if this is the first node
                self.least_recent_idx = Some(new_idx);
            }
        }

        self.most_recent_idx = Some(new_idx);

        match self.map.entry(key) {
            Entry::Occupied(mut occupied) => {
                let old = occupied.insert(RecentlyUsedItem {
                    node_idx: new_idx,
                    value,
                });

                // Remove the old occurrence of this key
                self.remove_node(old.node_idx);
            }
            Entry::Vacant(vacant) => {
                vacant.insert(RecentlyUsedItem {
                    node_idx: new_idx,
                    value,
                });
            }
        }
    }

    fn remove_node(&mut self, idx: usize) {
        match self.nodes[idx] {
            Node::Value {
                prev_idx, next_idx, ..
            } => {
                match prev_idx {
                    Some(prev) => match &mut self.nodes[prev] {
                        Node::Value {
                            next_idx: old_next_idx,
                            ..
                        } => {
                            *old_next_idx = next_idx;
                        }
                        Node::Free { .. } => unreachable!(),
                    },
                    None => {
                        // This node was the least recent
                        self.least_recent_idx = next_idx;
                    }
                }

                match next_idx {
                    Some(next) => match &mut self.nodes[next] {
                        Node::Value {
                            prev_idx: old_prev_idx,
                            ..
                        } => {
                            *old_prev_idx = prev_idx;
                        }
                        Node::Free { .. } => unreachable!(),
                    },
                    None => {
                        // This node was the most recent
                        self.most_recent_idx = prev_idx;
                    }
                }
            }
            Node::Free { .. } => unreachable!(),
        }

        // Add to free list
        self.nodes[idx] = Node::Free {
            next_idx: self.free,
        };
        self.free = Some(idx);
    }

    pub fn remove<Q: ?Sized>(&mut self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq,
    {
        self.map.remove(key.borrow()).map(|entry| {
            self.remove_node(entry.node_idx);
            entry.value
        })
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

    pub fn pop(&mut self) -> Option<(K, V)> {
        let least_recent_idx = self.least_recent_idx?;
        let least_recent_node = &self.nodes[least_recent_idx];

        match *least_recent_node {
            Node::Value { value: key, .. } => {
                let value = self.remove(&key);
                value.map(|v| (key, v))
            }
            Node::Free { .. } => unreachable!(),
        }
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
        rus.insert("c", ());
        assert_eq!(rus.pop(), Some(("a", ())));
        assert_eq!(rus.pop(), Some(("b", ())));
        assert_eq!(rus.pop(), Some(("c", ())));
        assert_eq!(rus.pop(), None);
    }

    #[test]
    fn reinsert() {
        let mut rus = RecentlyUsedMap::new();
        rus.insert("a", ());
        rus.insert("b", ());
        rus.insert("c", ());
        rus.insert("a", ());
        assert_eq!(rus.pop(), Some(("b", ())));
        assert_eq!(rus.pop(), Some(("c", ())));
        assert_eq!(rus.pop(), Some(("a", ())));
        assert_eq!(rus.pop(), None);
    }

    #[test]
    fn remove() {
        let mut rus = RecentlyUsedMap::new();
        rus.insert("a", ());
        rus.insert("b", ());
        rus.remove("a");
        rus.remove("c");
        assert_eq!(rus.pop(), Some(("b", ())));
        assert_eq!(rus.pop(), None);
    }

    #[test]
    fn reuses_free_nodes() {
        let mut rus = RecentlyUsedMap::new();
        rus.insert("a", ());
        rus.insert("b", ());
        rus.insert("c", ());
        rus.remove("b");
        rus.remove("c");
        rus.remove("a");
        rus.insert("d", ());
        rus.insert("e", ());
        rus.insert("f", ());
        assert_eq!(rus.nodes.len(), 3);
    }
}
