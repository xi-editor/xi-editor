// Copyright 2018 The xi-editor Authors.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::cmp::{self, Ordering};
use std::collections::vec_deque::{Drain, IntoIter, Iter, IterMut, VecDeque};
use std::hash::{Hash, Hasher};
use std::ops::{Index, IndexMut, RangeBounds};

/// Provides fixed size ring buffer that overwrites elements in FIFO order on
/// insertion when full.  API provided is similar to VecDeque & uses a VecDeque
/// internally. One distinction is that only append-like insertion is allowed.
/// This means that insert & push_front are not allowed.  The reasoning is that
/// there is ambiguity on how such functions should operate since it would be
/// pretty impossible to maintain a FIFO ordering.
///
/// All operations that would cause growth beyond the limit drop the appropriate
/// number of elements from the front.  For example, on a full buffer push_front
/// replaces the first element.
///
/// The removal of elements on operation that would cause excess beyond the
/// limit happens first to make sure the space is available in the underlying
/// VecDeque, thus guaranteeing O(1) operations always.
#[derive(Clone, Debug)]
pub struct FixedLifoDeque<T> {
    storage: VecDeque<T>,
    limit: usize,
}

impl<T> FixedLifoDeque<T> {
    /// Constructs a ring buffer that will reject all insertions as no-ops.
    /// This also construct the underlying VecDeque with_capacity(0) which
    /// in the current stdlib implementation allocates 2 Ts.
    #[inline]
    pub fn new() -> Self {
        FixedLifoDeque::with_limit(0)
    }

    /// Constructs a fixed size ring buffer with the given number of elements.
    /// Attempts to insert more than this number of elements will cause excess
    /// elements to first be evicted in FIFO order (i.e. from the front).
    pub fn with_limit(n: usize) -> Self {
        FixedLifoDeque { storage: VecDeque::with_capacity(n), limit: n }
    }

    /// This sets a new limit on the container.  Excess elements are dropped in
    /// FIFO order.  The new capacity is reset to the requested limit which will
    /// likely result in re-allocation + copies/clones even if the limit
    /// shrinks.
    pub fn reset_limit(&mut self, n: usize) {
        if n < self.limit {
            let overflow = self.limit - n;
            self.drop_excess_for_inserting(overflow);
        }
        self.limit = n;
        self.storage.reserve_exact(n);
        self.storage.shrink_to_fit();
        debug_assert!(self.storage.len() <= self.limit);
    }

    /// Returns the current limit this ring buffer is configured with.
    #[inline]
    pub fn limit(&self) -> usize {
        self.limit
    }

    #[inline]
    pub fn get(&self, index: usize) -> Option<&T> {
        self.storage.get(index)
    }

    #[inline]
    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.storage.get_mut(index)
    }

    #[inline]
    pub fn swap(&mut self, i: usize, j: usize) {
        self.storage.swap(i, j);
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.limit
    }

    #[inline]
    pub fn iter(&self) -> Iter<T> {
        self.storage.iter()
    }

    #[inline]
    pub fn iter_mut(&mut self) -> IterMut<T> {
        self.storage.iter_mut()
    }

    /// Returns a tuple of 2 slices that represents the ring buffer. [0] is the
    /// beginning of the buffer to the physical end of the array or the last
    /// element (whichever comes first).  [1] is the continuation of [0] if the
    /// ring buffer has wrapped the contiguous storage.
    #[inline]
    pub fn as_slices(&self) -> (&[T], &[T]) {
        self.storage.as_slices()
    }

    #[inline]
    pub fn as_mut_slices(&mut self) -> (&mut [T], &mut [T]) {
        self.storage.as_mut_slices()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.storage.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.storage.is_empty()
    }

    #[inline]
    pub fn drain<R>(&mut self, range: R) -> Drain<T>
    where
        R: RangeBounds<usize>,
    {
        self.storage.drain(range)
    }

    #[inline]
    pub fn clear(&mut self) {
        self.storage.clear();
    }

    #[inline]
    pub fn contains(&self, x: &T) -> bool
    where
        T: PartialEq<T>,
    {
        self.storage.contains(x)
    }

    #[inline]
    pub fn front(&self) -> Option<&T> {
        self.storage.front()
    }

    #[inline]
    pub fn front_mut(&mut self) -> Option<&mut T> {
        self.storage.front_mut()
    }

    #[inline]
    pub fn back(&self) -> Option<&T> {
        self.storage.back()
    }

    #[inline]
    pub fn back_mut(&mut self) -> Option<&mut T> {
        self.storage.back_mut()
    }

    #[inline]
    fn drop_excess_for_inserting(&mut self, n_to_be_inserted: usize) {
        if self.storage.len() + n_to_be_inserted > self.limit {
            let overflow =
                self.storage.len().min(self.storage.len() + n_to_be_inserted - self.limit);
            self.storage.drain(..overflow);
        }
    }

    /// Always an O(1) operation.  Memory is never reclaimed.
    #[inline]
    pub fn pop_front(&mut self) -> Option<T> {
        self.storage.pop_front()
    }

    /// Always an O(1) operation.  If the number of elements is at the limit,
    /// the element at the front is overwritten.
    ///
    /// Post condition: The number of elements is <= limit
    pub fn push_back(&mut self, value: T) {
        self.drop_excess_for_inserting(1);
        self.storage.push_back(value);
        // For when limit == 0
        self.drop_excess_for_inserting(0);
    }

    /// Always an O(1) operation.  Memory is never reclaimed.
    #[inline]
    pub fn pop_back(&mut self) -> Option<T> {
        self.storage.pop_back()
    }

    #[inline]
    pub fn swap_remove_back(&mut self, index: usize) -> Option<T> {
        self.storage.swap_remove_back(index)
    }

    #[inline]
    pub fn swap_remove_front(&mut self, index: usize) -> Option<T> {
        self.storage.swap_remove_front(index)
    }

    /// Always an O(1) operation.
    #[inline]
    pub fn remove(&mut self, index: usize) -> Option<T> {
        self.storage.remove(index)
    }

    pub fn split_off(&mut self, at: usize) -> FixedLifoDeque<T> {
        FixedLifoDeque { storage: self.storage.split_off(at), limit: self.limit }
    }

    /// Always an O(m) operation where m is the length of `other'.
    pub fn append(&mut self, other: &mut VecDeque<T>) {
        self.drop_excess_for_inserting(other.len());
        self.storage.append(other);
        // For when limit == 0
        self.drop_excess_for_inserting(0);
    }

    #[inline]
    pub fn retain<F>(&mut self, f: F)
    where
        F: FnMut(&T) -> bool,
    {
        self.storage.retain(f);
    }
}

impl<T: Clone> FixedLifoDeque<T> {
    /// Resizes a fixed queue.  This doesn't change the limit so the resize is
    /// capped to the limit.  Additionally, resizing drops the elements from the
    /// front unlike with a regular VecDeque.
    pub fn resize(&mut self, new_len: usize, value: T) {
        if new_len < self.len() {
            let to_drop = self.len() - new_len;
            self.storage.drain(..to_drop);
        } else {
            self.storage.resize(cmp::min(self.limit, new_len), value);
        }
    }
}

impl<A: PartialEq> PartialEq for FixedLifoDeque<A> {
    #[inline]
    fn eq(&self, other: &FixedLifoDeque<A>) -> bool {
        self.storage == other.storage
    }
}

impl<A: Eq> Eq for FixedLifoDeque<A> {}

impl<A: PartialOrd> PartialOrd for FixedLifoDeque<A> {
    #[inline]
    fn partial_cmp(&self, other: &FixedLifoDeque<A>) -> Option<Ordering> {
        self.storage.partial_cmp(&other.storage)
    }
}

impl<A: Ord> Ord for FixedLifoDeque<A> {
    #[inline]
    fn cmp(&self, other: &FixedLifoDeque<A>) -> Ordering {
        self.storage.cmp(&other.storage)
    }
}

impl<A: Hash> Hash for FixedLifoDeque<A> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.storage.hash(state);
    }
}

impl<A> Index<usize> for FixedLifoDeque<A> {
    type Output = A;

    #[inline]
    fn index(&self, index: usize) -> &A {
        &self.storage[index]
    }
}

impl<A> IndexMut<usize> for FixedLifoDeque<A> {
    #[inline]
    fn index_mut(&mut self, index: usize) -> &mut A {
        &mut self.storage[index]
    }
}

impl<T> IntoIterator for FixedLifoDeque<T> {
    type Item = T;
    type IntoIter = IntoIter<T>;

    /// Consumes the list into a front-to-back iterator yielding elements by
    /// value.
    #[inline]
    fn into_iter(self) -> IntoIter<T> {
        self.storage.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a FixedLifoDeque<T> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;

    #[inline]
    fn into_iter(self) -> Iter<'a, T> {
        self.storage.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut FixedLifoDeque<T> {
    type Item = &'a mut T;
    type IntoIter = IterMut<'a, T>;

    #[inline]
    fn into_iter(self) -> IterMut<'a, T> {
        self.storage.iter_mut()
    }
}

impl<A> Extend<A> for FixedLifoDeque<A> {
    fn extend<T: IntoIterator<Item = A>>(&mut self, iter: T) {
        for elt in iter {
            self.push_back(elt);
        }
    }
}

impl<'a, T: 'a + Copy> Extend<&'a T> for FixedLifoDeque<T> {
    fn extend<I: IntoIterator<Item = &'a T>>(&mut self, iter: I) {
        self.extend(iter.into_iter().cloned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "benchmarks")]
    use test::Bencher;

    #[test]
    fn test_basic_insertions() {
        let mut tester = FixedLifoDeque::with_limit(3);
        assert_eq!(tester.len(), 0);
        assert_eq!(tester.capacity(), 3);
        assert_eq!(tester.front(), None);
        assert_eq!(tester.back(), None);

        tester.push_back(1);
        assert_eq!(tester.len(), 1);
        assert_eq!(tester.front(), Some(1).as_ref());
        assert_eq!(tester.back(), Some(1).as_ref());

        tester.push_back(2);
        assert_eq!(tester.len(), 2);
        assert_eq!(tester.front(), Some(1).as_ref());
        assert_eq!(tester.back(), Some(2).as_ref());

        tester.push_back(3);
        tester.push_back(4);
        assert_eq!(tester.len(), 3);
        assert_eq!(tester.front(), Some(2).as_ref());
        assert_eq!(tester.back(), Some(4).as_ref());
        assert_eq!(tester[0], 2);
        assert_eq!(tester[1], 3);
        assert_eq!(tester[2], 4);
    }

    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_push_back(b: &mut Bencher) {
        let mut q = FixedLifoDeque::with_limit(10);
        b.iter(|| q.push_back(5));
    }

    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_deletion_from_empty(b: &mut Bencher) {
        let mut q = FixedLifoDeque::<u32>::with_limit(10000);
        b.iter(|| q.pop_front());
    }

    #[cfg(feature = "benchmarks")]
    #[bench]
    fn bench_deletion_from_non_empty(b: &mut Bencher) {
        let mut q = FixedLifoDeque::with_limit(1000000);
        for i in 0..q.limit() {
            q.push_back(i);
        }
        b.iter(|| q.pop_front());
    }
}
