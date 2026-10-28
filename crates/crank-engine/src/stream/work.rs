//! A test-only count of the hub's work units (#293), so the cost of a tick is asserted
//! as a number, never timed. In a normal build [`Work`] is zero-sized and every method is
//! a no-op.
use std::collections::BTreeMap;

/// Shared work counter: cloned into every stream and flow of a hub.
#[derive(Clone, Default)]
pub(crate) struct Work {
    #[cfg(test)]
    units: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
impl Work {
    /// Count `n` units of work.
    #[cfg_attr(not(test), allow(clippy::unused_self))]
    pub(crate) fn add(&self, n: usize) {
        #[cfg(test)]
        self.units
            .fetch_add(n, std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(test))]
        let _ = n;
    }
    /// Units counted so far.
    #[cfg(test)]
    pub(crate) fn total(&self) -> usize {
        self.units.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// A map whose every visited element counts as a unit of work: any pass over it, reading
/// or writing, shows up in the count. Point operations (`get`, `insert`, `remove`) are one
/// unit each.
pub(crate) struct Counted<K, V> {
    map: BTreeMap<K, V>,
    work: Work,
}
impl<K: Ord, V> Counted<K, V> {
    pub(crate) fn new(work: Work) -> Self {
        Self {
            map: BTreeMap::new(),
            work,
        }
    }
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
    pub(crate) fn contains_key(&self, k: &K) -> bool {
        self.work.add(1);
        self.map.contains_key(k)
    }
    #[cfg(test)]
    pub(crate) fn get(&self, k: &K) -> Option<&V> {
        self.work.add(1);
        self.map.get(k)
    }
    pub(crate) fn insert(&mut self, k: K, v: V) -> Option<V> {
        self.work.add(1);
        self.map.insert(k, v)
    }
    pub(crate) fn remove(&mut self, k: &K) -> Option<V> {
        self.work.add(1);
        self.map.remove(k)
    }
    pub(crate) fn values(&self) -> impl Iterator<Item = &V> {
        let work = self.work.clone();
        self.map.values().inspect(move |_| work.add(1))
    }
    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = (&K, &mut V)> {
        let work = self.work.clone();
        self.map.iter_mut().inspect(move |_| work.add(1))
    }
}
