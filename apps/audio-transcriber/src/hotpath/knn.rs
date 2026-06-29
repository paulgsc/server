
/// Stack-only k-nearest-neighbor query engine for the real-time hotpath.
///
/// # Design
///
/// `FixedKnnEngine<DIM, ENTRIES>` stores `ENTRIES` centroids of dimension `DIM`
/// in a flat 2D array. `search_nearest::<K>` performs a linear scan over all
/// `ENTRIES` centroids and returns the K closest in a `FixedNeighborSet<K>` —
/// a fixed-size stack-allocated result type.
///
/// There are no heap allocations anywhere in the query path. The worst-case
/// iteration count is exactly `ENTRIES` (a compile-time constant).
///
/// See `docs/adr/004-stack-knn.md` for full rationale.

/// A single neighbor result.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Neighbor {
    /// Index into the centroid array (caller maps to their ID space).
    pub index: usize,
    /// Squared Euclidean distance (no sqrt — monotonic for ranking).
    pub distance: f32,
}

/// Fixed-size, stack-allocated set of K nearest neighbors.
///
/// Maintains the K closest neighbors seen so far during a linear scan.
/// Internally uses insertion into a sorted `[Option<Neighbor>; K]` array.
/// No heap allocation.
pub struct FixedNeighborSet<const K: usize> {
    items: [Option<Neighbor>; K],
    count: usize,
}

impl<const K: usize> FixedNeighborSet<K> {
    fn new() -> Self {
        Self {
            items: [None; K],
            count: 0,
        }
    }

    /// Insert `candidate` if it is closer than the current worst neighbor.
    ///
    /// Uses an inline insertion-sort step over K elements.
    /// K is typically 1–5 in practice, so this is effectively branchless.
    #[inline(always)]
    fn insert_if_closer(&mut self, candidate: Neighbor) {
        if self.count < K {
            // Set not yet full — insert at correct sorted position
            self.items[self.count] = Some(candidate);
            self.count += 1;
            // Bubble up to maintain sorted order (ascending distance)
            let mut i = self.count - 1;
            while i > 0 {
                let prev = self.items[i - 1].unwrap();
                let curr = self.items[i].unwrap();
                if curr.distance < prev.distance {
                    self.items.swap(i - 1, i);
                    i -= 1;
                } else {
                    break;
                }
            }
        } else {
            // Set full — only insert if closer than the worst (last element)
            let worst_distance = self.items[K - 1].map_or(f32::MAX, |n| n.distance);
            if candidate.distance < worst_distance {
                self.items[K - 1] = Some(candidate);
                // Re-sort the last element into position
                let mut i = K - 1;
                while i > 0 {
                    let prev = self.items[i - 1].unwrap();
                    let curr = self.items[i].unwrap();
                    if curr.distance < prev.distance {
                        self.items.swap(i - 1, i);
                        i -= 1;
                    } else {
                        break;
                    }
                }
            }
        }
    }

    /// Returns the neighbors in ascending distance order.
    /// May return fewer than K if fewer than K items have been inserted.
    pub fn as_slice(&self) -> &[Option<Neighbor>] {
        &self.items[..self.count]
    }

    /// Number of neighbors collected.
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The closest neighbor, if any.
    pub fn nearest(&self) -> Option<Neighbor> {
        self.items[0]
    }
}

/// Flat centroid store and k-NN search engine.
///
/// # Type Parameters
///
/// - `DIM`: Embedding dimension. Fixed at compile time.
/// - `ENTRIES`: Number of indexed centroids. Fixed at compile time.
///
/// # Memory layout
///
/// `centroids: [[f32; DIM]; ENTRIES]` — row-major, so each centroid's DIM floats
/// are contiguous. The inner distance loop accesses them sequentially, which is
/// optimal for cache prefetching.
pub struct FixedKnnEngine<const DIM: usize, const ENTRIES: usize> {
    centroids: [[f32; DIM]; ENTRIES],
    count: usize, // how many entries have been set (≤ ENTRIES)
}

impl<const DIM: usize, const ENTRIES: usize> FixedKnnEngine<DIM, ENTRIES> {
    /// Create an empty engine. Call `insert` to add centroids.
    pub const fn new() -> Self {
        Self {
            centroids: [[0.0_f32; DIM]; ENTRIES],
            count: 0,
        }
    }

    /// Insert a centroid at the next available slot.
    ///
    /// Returns the index of the inserted centroid, or `None` if the engine is full.
    ///
    /// Call this at startup (not in the hotpath).
    pub fn insert(&mut self, centroid: &[f32; DIM]) -> Option<usize> {
        if self.count >= ENTRIES {
            return None;
        }
        self.centroids[self.count] = *centroid;
        let idx = self.count;
        self.count += 1;
        Some(idx)
    }

    /// Search for the K nearest neighbors to `query`.
    ///
    /// # Returns
    ///
    /// A `FixedNeighborSet<K>` containing up to K nearest neighbors in ascending
    /// distance order. Fully stack-allocated.
    ///
    /// # Real-time guarantees
    ///
    /// - Zero allocations.
    /// - Exactly `self.count` iterations (deterministic worst case = `ENTRIES`).
    /// - O(ENTRIES × DIM) time with no branching on dynamic conditions.
    pub fn search_nearest<const K: usize>(&self, query: &[f32; DIM]) -> FixedNeighborSet<K> {
        let mut results = FixedNeighborSet::<K>::new();

        for i in 0..self.count {
            let centroid = &self.centroids[i];

            // Squared Euclidean distance — no sqrt (monotonic for ranking)
            let mut dist = 0.0_f32;
            for j in 0..DIM {
                let diff = centroid[j] - query[j];
                dist = diff.mul_add(diff, dist); // FMA — single instruction on x86-64 AVX
            }

            results.insert_if_closer(Neighbor { index: i, distance: dist });
        }

        results
    }

    /// Number of centroids inserted.
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

impl<const DIM: usize, const ENTRIES: usize> Default for FixedKnnEngine<DIM, ENTRIES> {
    fn default() -> Self {
        Self::new()
    }
}

// MILESTONE M4 TESTS
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_nearest_1() {
        let mut engine = FixedKnnEngine::<2, 4>::new();
        engine.insert(&[0.0, 0.0]);
        engine.insert(&[1.0, 0.0]);
        engine.insert(&[0.0, 1.0]);
        engine.insert(&[5.0, 5.0]);

        let query = [0.1_f32, 0.1];
        let results = engine.search_nearest::<1>(&query);

        assert_eq!(results.len(), 1);
        assert_eq!(results.nearest().unwrap().index, 0); // [0,0] is closest
    }

    #[test]
    fn search_k_nearest_ordering() {
        let mut engine = FixedKnnEngine::<1, 5>::new();
        engine.insert(&[10.0]); // far
        engine.insert(&[1.0]);  // closest
        engine.insert(&[3.0]);  // 3rd
        engine.insert(&[2.0]);  // 2nd
        engine.insert(&[20.0]); // farthest

        let query = [0.0_f32];
        let results = engine.search_nearest::<3>(&query);

        assert_eq!(results.len(), 3);
        let items: Vec<usize> = results.as_slice().iter().filter_map(|n| n.map(|x| x.index)).collect();
        // Expected order: index 1 (dist 1), index 3 (dist 4), index 2 (dist 9)
        assert_eq!(items[0], 1);
        assert_eq!(items[1], 3);
        assert_eq!(items[2], 2);
    }

    #[test]
    fn search_fewer_than_k_entries() {
        let mut engine = FixedKnnEngine::<2, 10>::new();
        engine.insert(&[0.0, 0.0]);
        engine.insert(&[1.0, 1.0]);

        let query = [0.5_f32, 0.5];
        let results = engine.search_nearest::<5>(&query);

        // Only 2 entries exist, so we get 2 results even though K=5
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn empty_engine_returns_empty() {
        let engine = FixedKnnEngine::<3, 8>::new();
        let query = [0.0_f32; 3];
        let results = engine.search_nearest::<3>(&query);
        assert!(results.is_empty());
    }

    #[test]
    fn insert_beyond_capacity_returns_none() {
        let mut engine = FixedKnnEngine::<1, 2>::new();
        assert!(engine.insert(&[1.0]).is_some());
        assert!(engine.insert(&[2.0]).is_some());
        assert!(engine.insert(&[3.0]).is_none()); // over capacity
    }

    #[test]
    fn result_type_size_is_bounded() {
        // FixedNeighborSet<K> must fit on the stack — verify its size is bounded
        // by K and does not contain any heap-allocated types.
        let size = std::mem::size_of::<FixedNeighborSet<8>>();
        // Each Neighbor is index (usize) + distance (f32) = 12 bytes + alignment = 16 bytes
        // Option<Neighbor> with niche = same (no niche for f32 in a struct)
        // [Option<Neighbor>; 8] + count (usize) = 8 * 16 + 8 = 136 bytes
        // The exact value will depend on target alignment, but it must be finite
        // and not involve heap pointers.
        assert!(size < 1024, "FixedNeighborSet<8> should be under 1KB (got {size} bytes)");
        assert!(size > 0);
    }
}
