//! Shared waiter bookkeeping for the wake-and-retry resources.
//!
//! `Resource`, `PriorityResource` (and the post-MVP `PreemptiveResource`) all
//! follow the same protocol: a request takes a free unit immediately if one is
//! available, otherwise it parks a waker in an ordered queue; releasing a unit
//! wakes the next live waiter, which re-polls and acquires. The only thing that
//! differs between them is the *order* in which parked waiters are served.
//!
//! [`WaitQueue`] captures that protocol once. Waiters are served in ascending
//! `(key, seq)` order — smallest key first, ties broken FIFO by an internal
//! monotonic sequence. FIFO scheduling is therefore just the degenerate case
//! `K = ()` (every key equal → pure insertion order); priority scheduling uses
//! `K = u32` (lower number = higher priority).
//!
//! Extracting this here keeps the `registered` / `canceled` / drop-cancel
//! invariants (see `SPEC.md §4.7`) in a single place rather than copied across
//! every resource type.

use std::cell::Cell;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::rc::Rc;
use std::task::Waker;

/// One suspended requester parked in a [`WaitQueue`].
struct Entry<K> {
    key: K,
    /// Monotonic tie-breaker guaranteeing FIFO order within equal keys.
    seq: u64,
    waker: Waker,
    /// Shared with the owning request future. Its `Drop` sets this to `true`
    /// when abandoned before being granted; [`WaitQueue::release`] skips such
    /// entries so live waiters behind them are still served.
    canceled: Rc<Cell<bool>>,
}

// `BinaryHeap` is a max-heap, but we want the *smallest* `(key, seq)` served
// first, so every comparison is reversed (compare `other` to `self`).
impl<K: Ord> Ord for Entry<K> {
    fn cmp(&self, other: &Self) -> Ordering {
        other.key.cmp(&self.key).then(other.seq.cmp(&self.seq))
    }
}

impl<K: Ord> PartialOrd for Entry<K> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<K: Ord> PartialEq for Entry<K> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.seq == other.seq
    }
}

impl<K: Ord> Eq for Entry<K> {}

/// A capacity-limited pool plus its ordered queue of blocked waiters.
///
/// Generic over the ordering key `K`: `WaitQueue<()>` is FIFO, `WaitQueue<u32>`
/// is priority-ordered (lower key first, FIFO within a level). All methods
/// operate on `&mut self`; the resource wraps this in `Rc<RefCell<…>>` and
/// shares it by cloning, exactly like the rest of the crate.
pub(crate) struct WaitQueue<K: Ord> {
    capacity: usize,
    in_use: usize,
    /// Monotonic counter assigning each waiter its FIFO tie-breaker.
    next_seq: u64,
    waiters: BinaryHeap<Entry<K>>,
}

impl<K: Ord> WaitQueue<K> {
    /// Create a pool with the given capacity.
    ///
    /// Callers (the resource constructors) are responsible for rejecting a
    /// zero capacity with a type-specific panic message before calling this.
    pub(crate) fn new(capacity: usize) -> Self {
        WaitQueue {
            capacity,
            in_use: 0,
            next_seq: 0,
            waiters: BinaryHeap::new(),
        }
    }

    /// Units currently held.
    pub(crate) fn in_use(&self) -> usize {
        self.in_use
    }

    /// Total capacity.
    pub(crate) fn capacity(&self) -> usize {
        self.capacity
    }

    /// Take a free unit if one is available, returning whether it was granted.
    ///
    /// This is called at the top of every request poll — including re-polls of
    /// an already-registered waiter — so a woken waiter acquires the unit that
    /// [`release`](WaitQueue::release) freed for it. Because `release` wakes
    /// only the single next-in-line waiter, the unconditional grab here cannot
    /// jump the queue.
    pub(crate) fn try_acquire(&mut self) -> bool {
        if self.in_use < self.capacity {
            self.in_use += 1;
            true
        } else {
            false
        }
    }

    /// Park a waiter in the queue. Call only once per request (guarded by the
    /// request's `registered` flag) to avoid double-queuing across polls.
    pub(crate) fn register(&mut self, key: K, waker: Waker, canceled: Rc<Cell<bool>>) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.waiters.push(Entry {
            key,
            seq,
            waker,
            canceled,
        });
    }

    /// Release one held unit and wake the next live waiter, if any.
    ///
    /// Canceled entries (abandoned requests) are popped and discarded without
    /// waking, so the first *live* waiter in priority/FIFO order is the one
    /// resumed — no starvation behind a dead entry.
    pub(crate) fn release(&mut self) {
        self.in_use -= 1;
        while let Some(entry) = self.waiters.pop() {
            if !entry.canceled.get() {
                entry.waker.wake();
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
    use std::sync::Arc;
    use std::task::Wake;

    /// A waker that records, into a shared counter, the order in which wakers
    /// fire. Each `RecordingWaker` stamps its `slot` with the next tick so the
    /// test can assert *which* waiter was resumed.
    struct RecordingWaker {
        order: Arc<AtomicUsize>,
        slot: Arc<AtomicUsize>,
    }

    impl Wake for RecordingWaker {
        fn wake(self: Arc<Self>) {
            let tick = self.order.fetch_add(1, AtomicOrdering::SeqCst) + 1;
            self.slot.store(tick, AtomicOrdering::SeqCst);
        }
    }

    /// Returns `(waker, slot)`. After the queue wakes it, `slot` holds a
    /// nonzero tick recording the relative wake order (1 = woken first).
    fn recording(order: &Arc<AtomicUsize>) -> (Waker, Arc<AtomicUsize>) {
        let slot = Arc::new(AtomicUsize::new(0));
        let waker = Waker::from(Arc::new(RecordingWaker {
            order: Arc::clone(order),
            slot: Arc::clone(&slot),
        }));
        (waker, slot)
    }

    fn live() -> Rc<Cell<bool>> {
        Rc::new(Cell::new(false))
    }

    #[test]
    fn try_acquire_respects_capacity() {
        let mut q: WaitQueue<()> = WaitQueue::new(2);
        assert!(q.try_acquire());
        assert!(q.try_acquire());
        assert!(!q.try_acquire()); // capacity exhausted
        assert_eq!(q.in_use(), 2);
        assert_eq!(q.capacity(), 2);
        q.release();
        assert_eq!(q.in_use(), 1);
        assert!(q.try_acquire());
    }

    #[test]
    fn fifo_order_for_unit_key() {
        let mut q: WaitQueue<()> = WaitQueue::new(1);
        assert!(q.try_acquire());

        let order = Arc::new(AtomicUsize::new(0));
        let (w0, s0) = recording(&order);
        let (w1, s1) = recording(&order);
        let (w2, s2) = recording(&order);
        q.register((), w0, live());
        q.register((), w1, live());
        q.register((), w2, live());

        // Each release wakes exactly one waiter, in insertion order; the woken
        // waiter then re-acquires the freed unit (the wake-and-retry handshake).
        q.release();
        assert!(q.try_acquire());
        q.release();
        assert!(q.try_acquire());
        q.release();
        assert!(q.try_acquire());
        assert_eq!(s0.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(s1.load(AtomicOrdering::SeqCst), 2);
        assert_eq!(s2.load(AtomicOrdering::SeqCst), 3);
    }

    #[test]
    fn priority_order_then_fifo_within_level() {
        let mut q: WaitQueue<u32> = WaitQueue::new(1);
        assert!(q.try_acquire());

        let order = Arc::new(AtomicUsize::new(0));
        // Register out of priority order; two share priority 5 (FIFO between them).
        let (w_lo_a, s_lo_a) = recording(&order); // prio 5, first
        let (w_hi, s_hi) = recording(&order); // prio 1, highest
        let (w_lo_b, s_lo_b) = recording(&order); // prio 5, second
        q.register(5, w_lo_a, live());
        q.register(1, w_hi, live());
        q.register(5, w_lo_b, live());

        q.release(); // highest priority (1) first
        assert!(q.try_acquire());
        q.release(); // then prio 5, FIFO: lo_a
        assert!(q.try_acquire());
        q.release(); // then prio 5: lo_b
        assert!(q.try_acquire());
        assert_eq!(s_hi.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(s_lo_a.load(AtomicOrdering::SeqCst), 2);
        assert_eq!(s_lo_b.load(AtomicOrdering::SeqCst), 3);
    }

    #[test]
    fn release_skips_canceled_waiter() {
        let mut q: WaitQueue<u32> = WaitQueue::new(1);
        assert!(q.try_acquire());

        let order = Arc::new(AtomicUsize::new(0));
        let (w_dead, s_dead) = recording(&order);
        let (w_live, s_live) = recording(&order);
        let dead_flag = live();
        q.register(0, w_dead, Rc::clone(&dead_flag)); // highest priority, but canceled
        q.register(1, w_live, live());

        dead_flag.set(true); // abandon the top-priority waiter

        q.release();
        // The canceled entry must be skipped without consuming the wake.
        assert_eq!(s_dead.load(AtomicOrdering::SeqCst), 0);
        assert_eq!(s_live.load(AtomicOrdering::SeqCst), 1);
    }

    #[test]
    fn release_with_only_canceled_waiters_wakes_nobody() {
        let mut q: WaitQueue<()> = WaitQueue::new(1);
        assert!(q.try_acquire());

        let order = Arc::new(AtomicUsize::new(0));
        let (w, s) = recording(&order);
        let flag = live();
        q.register((), w, Rc::clone(&flag));
        flag.set(true);

        q.release();
        assert_eq!(s.load(AtomicOrdering::SeqCst), 0); // never woken
        assert_eq!(q.in_use(), 0); // unit still returned
    }
}
