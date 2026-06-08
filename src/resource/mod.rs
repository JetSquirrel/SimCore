use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

pub mod container;
pub use container::{Container, ContainerGetRequest, ContainerPutRequest};

mod preemptive;
pub use preemptive::{PreemptiveGuard, PreemptiveRequest, PreemptiveResource};
pub mod priority;
pub use priority::{PriorityResource, PriorityResourceGuard, PriorityResourceRequest};

pub(crate) mod wait_queue;
use wait_queue::WaitQueue;

/// A cloneable handle to a capacity-limited resource pool.
///
/// Units are acquired by calling [`request`](Resource::request) and awaiting
/// the returned future. If no unit is available the calling process is
/// suspended and woken in FIFO order when one becomes free.
///
/// `Resource` wraps an `Rc<RefCell<>>` internally, so cloning is cheap and
/// all clones share the same pool. It is `!Send + !Sync` — consistent with
/// `SimEnv`.
///
/// FIFO ordering is the degenerate `WaitQueue<()>` case: every waiter shares
/// the same (unit) key, so they are served purely in insertion order.
#[derive(Clone)]
pub struct Resource {
    state: Rc<RefCell<WaitQueue<()>>>,
}

impl Resource {
    /// Create a new resource pool with the given capacity.
    ///
    /// # Panics
    /// Panics if `capacity` is zero.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "Resource capacity must be at least 1");
        Resource {
            state: Rc::new(RefCell::new(WaitQueue::new(capacity))),
        }
    }

    /// Request one unit. Resolves immediately if a unit is available,
    /// otherwise suspends the calling process until one is released.
    ///
    /// The returned [`ResourceGuard`] releases the unit when dropped.
    #[must_use = "futures do nothing unless awaited"]
    pub fn request(&self) -> ResourceRequest {
        ResourceRequest {
            state: Rc::clone(&self.state),
            registered: false,
            canceled: Rc::new(Cell::new(false)),
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
}

/// Future returned by [`Resource::request`].
///
/// Resolves to a [`ResourceGuard`] once a unit is acquired.
pub struct ResourceRequest {
    state: Rc<RefCell<WaitQueue<()>>>,
    /// Whether this request has already been enqueued in the wait queue.
    /// Prevents double-queuing on repeated polls.
    registered: bool,
    /// Shared with the queue entry; set to `true` on drop if the request was
    /// registered but never granted, so the release loop skips it.
    canceled: Rc<Cell<bool>>,
}

impl Future for ResourceRequest {
    type Output = ResourceGuard;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<ResourceGuard> {
        // Drop the borrow before writing self.registered to satisfy the borrow checker.
        {
            let mut state = self.state.borrow_mut();
            if state.try_acquire() {
                return Poll::Ready(ResourceGuard {
                    state: Rc::clone(&self.state),
                });
            }
            if !self.registered {
                state.register((), cx.waker().clone(), Rc::clone(&self.canceled));
            }
        }
        self.registered = true;
        Poll::Pending
    }
}

impl Drop for ResourceRequest {
    fn drop(&mut self) {
        if self.registered {
            self.canceled.set(true);
        }
    }
}

/// RAII guard that holds one unit of a [`Resource`].
///
/// The unit is released automatically when this value is dropped, waking the
/// next suspended requester (if any) in FIFO order.
pub struct ResourceGuard {
    state: Rc<RefCell<WaitQueue<()>>>,
}

impl Drop for ResourceGuard {
    fn drop(&mut self) {
        // Release one unit and wake the next live (non-canceled) waiter; the
        // WaitQueue skips abandoned entries so live waiters are not stranded.
        self.state.borrow_mut().release();
    }
}
