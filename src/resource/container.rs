use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

struct GetWaiter {
    amount: f64,
    waker: Waker,
    done: Rc<Cell<bool>>,
    /// Shared with the owning `ContainerGetRequest`. Set to `true` if the
    /// future is dropped before being granted; the cascade skips canceled
    /// entries so the level is not deducted for an abandoned request.
    canceled: Rc<Cell<bool>>,
}

struct PutWaiter {
    amount: f64,
    waker: Waker,
    done: Rc<Cell<bool>>,
    canceled: Rc<Cell<bool>>,
}

struct ContainerState {
    capacity: f64,
    level: f64,
    get_waiters: VecDeque<GetWaiter>,
    put_waiters: VecDeque<PutWaiter>,
}

// ---------------------------------------------------------------------------
// Wake cascade helpers
// ---------------------------------------------------------------------------

/// Drain as many head-of-queue get waiters as current level allows (FIFO).
/// Canceled entries (from abandoned requests) are skipped without touching level.
///
/// Returns `true` if at least one live waiter was serviced — i.e. the level
/// changed and the *other* queue may now have become unblocked.
fn wake_get_waiters(state: &mut ContainerState) -> bool {
    let mut serviced = false;
    while let Some(front) = state.get_waiters.front() {
        if front.canceled.get() {
            state.get_waiters.pop_front();
            continue;
        }
        if state.level >= front.amount {
            let w = state.get_waiters.pop_front().unwrap();
            state.level -= w.amount;
            w.done.set(true);
            w.waker.wake();
            serviced = true;
        } else {
            break; // FIFO: head is blocked, nobody behind it can proceed
        }
    }
    serviced
}

/// Drain as many head-of-queue put waiters as available space allows (FIFO).
/// Canceled entries are skipped without touching level.
///
/// Returns `true` if at least one live waiter was serviced.
fn wake_put_waiters(state: &mut ContainerState) -> bool {
    let mut serviced = false;
    while let Some(front) = state.put_waiters.front() {
        if front.canceled.get() {
            state.put_waiters.pop_front();
            continue;
        }
        if state.level + front.amount <= state.capacity {
            let w = state.put_waiters.pop_front().unwrap();
            state.level += w.amount;
            w.done.set(true);
            w.waker.wake();
            serviced = true;
        } else {
            break;
        }
    }
    serviced
}

/// Run get/put cascades until no more progress is possible.
///
/// Loops while either queue services a waiter, since satisfying a get frees
/// space (possibly unblocking a put) and satisfying a put adds material
/// (possibly unblocking a get). Termination is driven by whether any waiter
/// was actually serviced — never by a float-level comparison — so a pass whose
/// gets and puts net to a zero level change still triggers another iteration
/// when it leaves a newly-serviceable waiter behind. Each serviced waiter
/// removes an entry from a finite queue, so the loop always terminates.
fn trigger_cascade(state: &mut ContainerState) {
    loop {
        let serviced_get = wake_get_waiters(state);
        let serviced_put = wake_put_waiters(state);
        if !serviced_get && !serviced_put {
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A cloneable handle to a continuous-quantity resource (e.g., a tank of
/// liquid, a battery, an inventory of medication).
///
/// `put(amount)` adds material; `get(amount)` removes it.  Both operations
/// suspend the calling process when they cannot immediately complete:
///
/// - `get` suspends when the current level is below the requested amount.
/// - `put` suspends when adding the amount would exceed the container's capacity.
///
/// Waiters are served **FIFO** within each queue. All clones share the same
/// internal state (cheap `Rc` clone). `Container` is `!Send + !Sync`,
/// consistent with `SimEnv`.
#[derive(Clone)]
pub struct Container {
    state: Rc<RefCell<ContainerState>>,
}

impl Container {
    /// Create an **empty** container with the given capacity.
    ///
    /// # Panics
    /// Panics if `capacity <= 0`.
    #[must_use]
    pub fn empty(capacity: f64) -> Self {
        Self::new(capacity, 0.0)
    }

    /// Create a container with the given capacity and initial level.
    ///
    /// # Panics
    /// Panics if `capacity <= 0`, `initial_level < 0`, or
    /// `initial_level > capacity`.
    #[must_use]
    pub fn new(capacity: f64, initial_level: f64) -> Self {
        assert!(capacity > 0.0, "Container capacity must be positive");
        assert!(
            initial_level >= 0.0,
            "Container initial_level must be non-negative"
        );
        assert!(
            initial_level <= capacity,
            "Container initial_level must not exceed capacity"
        );
        Container {
            state: Rc::new(RefCell::new(ContainerState {
                capacity,
                level: initial_level,
                get_waiters: VecDeque::new(),
                put_waiters: VecDeque::new(),
            })),
        }
    }

    /// Current level (amount of material present).
    #[must_use]
    pub fn level(&self) -> f64 {
        self.state.borrow().level
    }

    /// Maximum capacity.
    #[must_use]
    pub fn capacity(&self) -> f64 {
        self.state.borrow().capacity
    }

    /// Add `amount` to the container.
    ///
    /// Resolves immediately if `level + amount <= capacity`; otherwise
    /// suspends until enough space is available.
    ///
    /// # Panics
    /// Panics if `amount <= 0`.
    #[must_use = "futures do nothing unless awaited"]
    pub fn put(&self, amount: f64) -> ContainerPutRequest {
        assert!(amount > 0.0, "Container::put amount must be positive");
        ContainerPutRequest {
            state: Rc::clone(&self.state),
            amount,
            registered: false,
            done: Rc::new(Cell::new(false)),
            canceled: Rc::new(Cell::new(false)),
        }
    }

    /// Remove `amount` from the container.
    ///
    /// Resolves immediately if `level >= amount`; otherwise suspends until
    /// enough material is available.
    ///
    /// # Panics
    /// Panics if `amount <= 0`.
    #[must_use = "futures do nothing unless awaited"]
    pub fn get(&self, amount: f64) -> ContainerGetRequest {
        assert!(amount > 0.0, "Container::get amount must be positive");
        ContainerGetRequest {
            state: Rc::clone(&self.state),
            amount,
            registered: false,
            done: Rc::new(Cell::new(false)),
            canceled: Rc::new(Cell::new(false)),
        }
    }
}

// ---------------------------------------------------------------------------
// ContainerPutRequest
// ---------------------------------------------------------------------------

/// Future returned by [`Container::put`].
pub struct ContainerPutRequest {
    state: Rc<RefCell<ContainerState>>,
    amount: f64,
    registered: bool,
    /// Shared with the `PutWaiter` entry; the cascade sets this to `true`
    /// before calling `waker.wake()`, so the next poll can return `Ready`
    /// without re-checking the level.
    done: Rc<Cell<bool>>,
    /// Shared with the `PutWaiter` entry; the request's `Drop` impl sets this
    /// to `true` if the future is abandoned before being granted, so the
    /// cascade skips the entry without adding level.
    canceled: Rc<Cell<bool>>,
}

impl Future for ContainerPutRequest {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // Cascade already committed our put — no need to touch level again.
        if self.done.get() {
            return Poll::Ready(());
        }
        {
            let mut state = self.state.borrow_mut();
            if !self.registered && state.level + self.amount <= state.capacity {
                state.level += self.amount;
                // Full cascade, not just wake_get_waiters: a woken get may drain
                // the level and free space for a blocked put-waiter behind it.
                // Using only wake_get_waiters here would strand that put-waiter.
                trigger_cascade(&mut state);
                return Poll::Ready(());
            }
            if !self.registered {
                state.put_waiters.push_back(PutWaiter {
                    amount: self.amount,
                    waker: cx.waker().clone(),
                    done: Rc::clone(&self.done),
                    canceled: Rc::clone(&self.canceled),
                });
            }
        }
        self.registered = true;
        Poll::Pending
    }
}

impl Drop for ContainerPutRequest {
    fn drop(&mut self) {
        // If we registered but never completed (cascade would have set
        // `done`), mark the queue entry canceled so the cascade skips it.
        if self.registered && !self.done.get() {
            self.canceled.set(true);
        }
    }
}

// ---------------------------------------------------------------------------
// ContainerGetRequest
// ---------------------------------------------------------------------------

/// Future returned by [`Container::get`].
pub struct ContainerGetRequest {
    state: Rc<RefCell<ContainerState>>,
    amount: f64,
    registered: bool,
    /// Shared with the `GetWaiter` entry; the cascade sets this to `true`
    /// before calling `waker.wake()`, so the next poll can return `Ready`.
    done: Rc<Cell<bool>>,
    /// Shared with the `GetWaiter` entry; the request's `Drop` impl sets this
    /// to `true` if the future is abandoned before being granted, so the
    /// cascade skips the entry without deducting level.
    canceled: Rc<Cell<bool>>,
}

impl Future for ContainerGetRequest {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // Cascade already committed our get.
        if self.done.get() {
            return Poll::Ready(());
        }
        {
            let mut state = self.state.borrow_mut();
            // Only take level immediately if we haven't yet registered as a
            // waiter — taking level out-of-turn would violate FIFO ordering.
            if !self.registered && state.level >= self.amount {
                state.level -= self.amount;
                trigger_cascade(&mut state);
                return Poll::Ready(());
            }
            if !self.registered {
                state.get_waiters.push_back(GetWaiter {
                    amount: self.amount,
                    waker: cx.waker().clone(),
                    done: Rc::clone(&self.done),
                    canceled: Rc::clone(&self.canceled),
                });
            }
        }
        self.registered = true;
        Poll::Pending
    }
}

impl Drop for ContainerGetRequest {
    fn drop(&mut self) {
        if self.registered && !self.done.get() {
            self.canceled.set(true);
        }
    }
}
