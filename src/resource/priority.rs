// SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Priority-scheduled resource pool.
//!
//! [`PriorityResource`] serves blocked waiters by priority level — lower
//! number first, FIFO within a level. For a pool whose *holders* can be
//! evicted by more urgent requests, see
//! [`PreemptiveResource`](crate::PreemptiveResource).

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use super::wait_queue::WaitQueue;

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
///
/// An urgent case overtakes a routine one that queued first:
///
/// ```
/// use std::cell::RefCell;
/// use std::rc::Rc;
/// use simcore::{SimEnv, PriorityResource};
///
/// let mut env = SimEnv::with_seed(0);
/// let doctor = PriorityResource::new(1);
/// let seen = Rc::new(RefCell::new(Vec::new()));
///
/// // Occupy the doctor until t = 1.
/// let h = env.handle();
/// let d = doctor.clone();
/// env.spawn(async move {
///     let _g = d.request(5).await;
///     h.timeout(1.0).await;
/// });
///
/// // Two patients queue while the doctor is busy — routine (10) arrives
/// // before urgent (0), but urgent is served first.
/// for (name, priority) in [("routine", 10), ("urgent", 0)] {
///     let d = doctor.clone();
///     let seen = Rc::clone(&seen);
///     env.spawn(async move {
///         let _g = d.request(priority).await;
///         seen.borrow_mut().push(name);
///     });
/// }
///
/// env.run();
/// assert_eq!(*seen.borrow(), ["urgent", "routine"]); // lower number wins
/// ```
///
/// Internally this is a `WaitQueue<u32>` (a `pub(crate)` helper): the
/// `u32` priority is the ordering key, and the queue's internal sequence
/// counter provides FIFO tie-breaking within a level.
#[derive(Clone)]
pub struct PriorityResource {
    state: Rc<RefCell<WaitQueue<u32>>>,
}

impl std::fmt::Debug for PriorityResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("PriorityResource");
        if let Ok(q) = self.state.try_borrow() {
            d.field("in_use", &q.in_use())
                .field("capacity", &q.capacity())
                .field("queue_len", &q.live_waiters());
        }
        d.finish_non_exhaustive()
    }
}

impl PriorityResource {
    /// Create a new priority resource pool with the given capacity.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is zero.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "PriorityResource capacity must be at least 1");
        PriorityResource {
            state: Rc::new(RefCell::new(WaitQueue::new(capacity))),
        }
    }

    /// Request one unit at the given `priority` (lower = higher priority).
    ///
    /// Resolves immediately if a unit is available; otherwise suspends the
    /// calling process and wakes it before any lower-priority waiter when a
    /// unit becomes free.
    ///
    /// The returned [`PriorityResourceGuard`] releases the unit when dropped.
    #[must_use = "futures do nothing unless awaited"]
    pub fn request(&self, priority: u32) -> PriorityResourceRequest {
        PriorityResourceRequest {
            state: Rc::clone(&self.state),
            priority,
            registered: false,
            consumed: false,
            canceled: Rc::new(Cell::new(false)),
            granted: Rc::new(Cell::new(false)),
        }
    }

    /// Number of units currently in use.
    #[must_use]
    pub fn in_use(&self) -> usize {
        self.state.borrow().in_use()
    }

    /// Total capacity of this resource pool.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.state.borrow().capacity()
    }

    /// Number of processes currently queued waiting for a unit, across all
    /// priority levels. Excludes abandoned (canceled) requests.
    #[must_use]
    pub fn queue_len(&self) -> usize {
        self.state.borrow().live_waiters()
    }
}

/// Future returned by [`PriorityResource::request`].
///
/// Resolves to a [`PriorityResourceGuard`] once a unit is acquired.
pub struct PriorityResourceRequest {
    state: Rc<RefCell<WaitQueue<u32>>>,
    priority: u32,
    /// Prevents double-queuing on repeated polls (same pattern as `ResourceRequest`).
    registered: bool,
    /// Set once a granted/acquired unit has become a guard; `Drop` then must
    /// not release (the guard owns the unit).
    consumed: bool,
    /// Shared with the queue entry; set to `true` on drop if the request was
    /// registered but never granted.
    canceled: Rc<Cell<bool>>,
    /// Shared with the queue entry; set to `true` by `WaitQueue::release` when
    /// the unit is handed directly to this request. Checked first in `poll`.
    granted: Rc<Cell<bool>>,
}

impl std::fmt::Debug for PriorityResourceRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PriorityResourceRequest")
            .field("priority", &self.priority)
            .field("registered", &self.registered)
            .field("granted", &self.granted.get())
            .finish_non_exhaustive()
    }
}

impl Future for PriorityResourceRequest {
    type Output = PriorityResourceGuard;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<PriorityResourceGuard> {
        // Direct handoff: a released unit was transferred to us (see `Resource`).
        if self.granted.get() {
            self.consumed = true;
            return Poll::Ready(PriorityResourceGuard {
                state: Rc::clone(&self.state),
            });
        }
        // Drop the borrow before writing self.registered to satisfy the borrow checker.
        let acquired = {
            let mut state = self.state.borrow_mut();
            if state.try_acquire() {
                true
            } else {
                if !self.registered {
                    state.register(
                        self.priority,
                        cx.waker().clone(),
                        Rc::clone(&self.canceled),
                        Rc::clone(&self.granted),
                    );
                }
                false
            }
        };
        if acquired {
            self.consumed = true;
            return Poll::Ready(PriorityResourceGuard {
                state: Rc::clone(&self.state),
            });
        }
        self.registered = true;
        Poll::Pending
    }
}

impl Drop for PriorityResourceRequest {
    fn drop(&mut self) {
        if self.consumed {
            return; // the guard owns the unit and will release it
        }
        if self.granted.get() {
            // Handed a unit but never consumed it — pass it on (see `Resource`).
            self.state.borrow_mut().release();
        } else if self.registered {
            self.canceled.set(true);
        }
    }
}

/// RAII guard that holds one unit of a [`PriorityResource`].
///
/// The unit is released automatically when this value is dropped, waking the
/// highest-priority suspended requester (if any).
pub struct PriorityResourceGuard {
    state: Rc<RefCell<WaitQueue<u32>>>,
}

impl std::fmt::Debug for PriorityResourceGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PriorityResourceGuard")
            .finish_non_exhaustive()
    }
}

impl Drop for PriorityResourceGuard {
    fn drop(&mut self) {
        // Release one unit and wake the highest-priority live waiter; the
        // WaitQueue skips canceled (abandoned) waiters automatically.
        self.state.borrow_mut().release();
    }
}
