use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

// ---------------------------------------------------------------------------
// Internal waiter entry stored in the priority heap
// ---------------------------------------------------------------------------

struct PriorityWaiter {
    priority: u32,
    /// Monotonically increasing counter — guarantees FIFO ordering within the
    /// same priority level.
    seq: u64,
    waker: Waker,
}

// BinaryHeap is a max-heap. We want the *lowest* (priority, seq) pair at the
// top, so we reverse the natural ordering.
impl Ord for PriorityWaiter {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .priority
            .cmp(&self.priority)
            .then(other.seq.cmp(&self.seq))
    }
}

impl PartialOrd for PriorityWaiter {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for PriorityWaiter {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority && self.seq == other.seq
    }
}

impl Eq for PriorityWaiter {}

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

struct PriorityResourceState {
    capacity: usize,
    in_use: usize,
    /// Monotonic counter for FIFO tie-breaking within the same priority level.
    next_seq: u64,
    waiters: BinaryHeap<PriorityWaiter>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// A cloneable handle to a capacity-limited resource pool with priority
/// scheduling.
///
/// Like [`Resource`](crate::Resource), units are acquired by calling
/// [`request`](PriorityResource::request) and awaiting the returned future.
/// Unlike `Resource`, waiters are served in **priority order**: the waiter with
/// the lowest priority number is served first. Within the same priority level,
/// waiters are served FIFO.
///
/// **Priority convention:** lower number = higher priority (`0` is highest).
///
/// `PriorityResource` wraps an `Rc<RefCell<>>` internally, so cloning is cheap
/// and all clones share the same pool. It is `!Send + !Sync` — consistent with
/// `SimEnv`.
#[derive(Clone)]
pub struct PriorityResource {
    state: Rc<RefCell<PriorityResourceState>>,
}

impl PriorityResource {
    /// Create a new priority resource pool with the given capacity.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is zero.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "PriorityResource capacity must be at least 1");
        PriorityResource {
            state: Rc::new(RefCell::new(PriorityResourceState {
                capacity,
                in_use: 0,
                next_seq: 0,
                waiters: BinaryHeap::new(),
            })),
        }
    }

    /// Request one unit at the given `priority` (lower = higher priority).
    ///
    /// Resolves immediately if a unit is available; otherwise suspends the
    /// calling process and wakes it before any lower-priority waiter when a
    /// unit becomes free.
    ///
    /// The returned [`PriorityResourceGuard`] releases the unit when dropped.
    pub fn request(&self, priority: u32) -> PriorityResourceRequest {
        PriorityResourceRequest {
            state: Rc::clone(&self.state),
            priority,
            seq: 0,
            registered: false,
        }
    }

    /// Number of units currently in use.
    pub fn in_use(&self) -> usize {
        self.state.borrow().in_use
    }

    /// Total capacity of this resource pool.
    pub fn capacity(&self) -> usize {
        self.state.borrow().capacity
    }
}

/// Future returned by [`PriorityResource::request`].
///
/// Resolves to a [`PriorityResourceGuard`] once a unit is acquired.
pub struct PriorityResourceRequest {
    state: Rc<RefCell<PriorityResourceState>>,
    priority: u32,
    /// Sequence number assigned on first registration for FIFO tie-breaking.
    /// Zero until the request is first enqueued.
    seq: u64,
    /// Prevents double-queuing on repeated polls (same pattern as `ResourceRequest`).
    registered: bool,
}

impl Future for PriorityResourceRequest {
    type Output = PriorityResourceGuard;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<PriorityResourceGuard> {
        // Compute whether we need to register, returning the assigned seq if so.
        // The RefMut must be dropped before we write back to `self`.
        let newly_registered_seq: Option<u64> = {
            let mut state = self.state.borrow_mut();

            if state.in_use < state.capacity {
                state.in_use += 1;
                return Poll::Ready(PriorityResourceGuard {
                    state: Rc::clone(&self.state),
                });
            }

            if !self.registered {
                let seq = state.next_seq;
                state.next_seq += 1;
                state.waiters.push(PriorityWaiter {
                    priority: self.priority,
                    seq,
                    waker: cx.waker().clone(),
                });
                Some(seq)
            } else {
                None
            }
        };

        if let Some(seq) = newly_registered_seq {
            self.seq = seq;
            self.registered = true;
        }

        Poll::Pending
    }
}

/// RAII guard that holds one unit of a [`PriorityResource`].
///
/// The unit is released automatically when this value is dropped, waking the
/// highest-priority suspended requester (if any).
pub struct PriorityResourceGuard {
    state: Rc<RefCell<PriorityResourceState>>,
}

impl Drop for PriorityResourceGuard {
    fn drop(&mut self) {
        let mut state = self.state.borrow_mut();
        state.in_use -= 1;
        if let Some(waiter) = state.waiters.pop() {
            waiter.waker.wake();
        }
    }
}
