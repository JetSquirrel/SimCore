use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

mod preemptive;
pub mod priority;
pub use priority::{PriorityResource, PriorityResourceGuard, PriorityResourceRequest};

struct ResourceState {
    capacity: usize,
    in_use: usize,
    waiters: VecDeque<Waker>,
}

/// A cloneable handle to a capacity-limited resource pool.
///
/// Units are acquired by calling [`request`](Resource::request) and awaiting
/// the returned future. If no unit is available the calling process is
/// suspended and woken in FIFO order when one becomes free.
///
/// `Resource` wraps an `Rc<RefCell<>>` internally, so cloning is cheap and
/// all clones share the same pool. It is `!Send + !Sync` — consistent with
/// `SimEnv`.
#[derive(Clone)]
pub struct Resource {
    state: Rc<RefCell<ResourceState>>,
}

impl Resource {
    /// Create a new resource pool with the given capacity.
    ///
    /// # Panics
    /// Panics if `capacity` is zero.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "Resource capacity must be at least 1");
        Resource {
            state: Rc::new(RefCell::new(ResourceState {
                capacity,
                in_use: 0,
                waiters: VecDeque::new(),
            })),
        }
    }

    /// Request one unit. Resolves immediately if a unit is available,
    /// otherwise suspends the calling process until one is released.
    ///
    /// The returned [`ResourceGuard`] releases the unit when dropped.
    pub fn request(&self) -> ResourceRequest {
        ResourceRequest {
            state: Rc::clone(&self.state),
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

/// Future returned by [`Resource::request`].
///
/// Resolves to a [`ResourceGuard`] once a unit is acquired.
pub struct ResourceRequest {
    state: Rc<RefCell<ResourceState>>,
    /// Whether this request has already been enqueued in `waiters`.
    /// Prevents double-queuing on repeated polls.
    registered: bool,
}

impl Future for ResourceRequest {
    type Output = ResourceGuard;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<ResourceGuard> {
        // Drop the RefMut before writing self.registered to satisfy the borrow checker.
        {
            let mut state = self.state.borrow_mut();
            if state.in_use < state.capacity {
                state.in_use += 1;
                return Poll::Ready(ResourceGuard {
                    state: Rc::clone(&self.state),
                });
            }
            if !self.registered {
                state.waiters.push_back(cx.waker().clone());
            }
        }
        self.registered = true;
        Poll::Pending
    }
}

/// RAII guard that holds one unit of a [`Resource`].
///
/// The unit is released automatically when this value is dropped, waking the
/// next suspended requester (if any) in FIFO order.
pub struct ResourceGuard {
    state: Rc<RefCell<ResourceState>>,
}

impl Drop for ResourceGuard {
    fn drop(&mut self) {
        let mut state = self.state.borrow_mut();
        state.in_use -= 1;
        if let Some(waker) = state.waiters.pop_front() {
            waker.wake();
        }
    }
}
