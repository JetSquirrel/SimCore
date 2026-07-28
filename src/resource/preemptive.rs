// SPDX-FileCopyrightText: 2026 Christoph Kuhmuench <christoph.kuhmuench@gmail.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `PreemptiveResource` — a priority resource whose in-use units can be
//! *preempted* by a higher-priority request.
//!
//! ## Cooperative-at-yield semantics
//!
//! A discrete-event executor cannot forcibly unwind a process that is suspended
//! on an unrelated future (e.g. a service `timeout`): the only lever it has is
//! the process's `Waker`, and waking it merely re-polls the same `Pending`
//! future. Truly preemptive cancellation would require unwinding arbitrary
//! suspended stacks, which Rust's async model does not permit from the outside.
//!
//! `PreemptiveResource` therefore delivers preemption the way SimPy delivers an
//! interrupt and the way all Rust async cancellation works: **at the victim's
//! next yield point.** When a higher-priority request preempts a holder, the
//! holder's unit is transferred away immediately *and* its preemption signal is
//! fired. A well-behaved victim races its work against that signal — see the
//! [`PreemptiveResource`] type docs for the full pattern.
//!
//! A victim that ignores its signal keeps running (it has already lost the unit
//! on the books, so it can no longer block anyone). This mirrors the fact that
//! a Rust future which never checks for cancellation simply runs to completion.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use crate::event::{new_event, EventAwaitable, EventTrigger};

use super::wait_queue::WaitQueue;

/// Bookkeeping for one unit that is currently held.
struct Holder {
    /// Priority the unit was acquired at (lower = higher priority). Used to
    /// pick a victim: a request preempts the lowest-priority holder whose
    /// priority is strictly worse (greater) than the request's.
    priority: u32,
    /// Unique id so a guard can find and remove exactly its own holder entry
    /// (priorities are not unique).
    id: u64,
    /// Fired when this holder is preempted. Shared with the guard's
    /// `EventAwaitable` so the victim can observe preemption at a yield point.
    trigger: Option<EventTrigger>,
    /// Mirrors `trigger`'s fired state for synchronous `is_preempted()` checks
    /// without consuming the awaitable.
    preempted: Rc<Cell<bool>>,
}

struct PreemptiveState {
    /// Capacity accounting + the priority-ordered queue of *blocked* requests.
    /// `in_use` here counts granted units; `holders.len()` mirrors it exactly.
    wq: WaitQueue<u32>,
    /// One entry per currently-held unit.
    holders: Vec<Holder>,
    /// Monotonic id source for holder entries.
    next_holder_id: u64,
}

impl PreemptiveState {
    /// Pick the holder to evict for an `incoming` request, or `None` if none
    /// can be preempted.
    ///
    /// The victim is the holder with the **lowest priority** (numerically
    /// greatest) whose priority is *strictly worse* than `incoming`. When
    /// several holders share that lowest priority, the **most recently
    /// acquired** one is chosen — it has made the least progress, so preempting
    /// it wastes the least work. `max_by_key` returns the last maximal element,
    /// and `holders` is kept in acquisition order, so this tie-break is
    /// deterministic.
    fn victim_index(&self, incoming: u32) -> Option<usize> {
        self.holders
            .iter()
            .enumerate()
            .filter(|(_, h)| h.priority > incoming)
            .max_by_key(|(_, h)| h.priority)
            .map(|(i, _)| i)
    }
}

/// A cloneable handle to a capacity-limited pool whose held units can be
/// **preempted** by higher-priority requests.
///
/// Like [`PriorityResource`](crate::PriorityResource), units are requested at a
/// priority (lower number = higher priority) and blocked waiters are served in
/// priority order. *Unlike* it, when every unit is in use a higher-priority
/// request does not wait behind the holders — it **evicts** the lowest-priority
/// holder whose priority is strictly worse than its own, taking that unit
/// immediately.
///
/// Preemption is cooperative-at-yield: a higher-priority request fires the
/// victim's [`preempted`](PreemptiveGuard::preempted) signal, and the victim is
/// expected to bail via `any_of![work, guard.preempted()]`. A well-behaved
/// holder races its work against that signal:
///
/// ```
/// use simu::{SimEnv, PreemptiveResource, any_of};
/// let mut env = SimEnv::with_seed(0);
/// let res = PreemptiveResource::new(1);
/// let h = env.handle();
/// let r = res.clone();
/// env.spawn(async move {
///     let guard = r.request(2).await;
///     let service = 10.0;
///     // Race the service time against a possible preemption.
///     any_of![h.timeout(service), guard.preempted()].await;
///     if guard.is_preempted() {
///         // Higher-priority work took the unit — abandon and clean up.
///         return;
///     }
///     // Completed normally; dropping the guard releases the unit.
/// });
/// env.run();
/// ```
///
/// `PreemptiveResource` wraps an `Rc<RefCell<>>` internally, so cloning is
/// cheap and all clones share the same pool. It is `!Send + !Sync`.
#[derive(Clone)]
pub struct PreemptiveResource {
    state: Rc<RefCell<PreemptiveState>>,
}

impl std::fmt::Debug for PreemptiveResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("PreemptiveResource");
        if let Ok(s) = self.state.try_borrow() {
            d.field("in_use", &s.wq.in_use())
                .field("capacity", &s.wq.capacity())
                .field("queue_len", &s.wq.live_waiters());
        }
        d.finish_non_exhaustive()
    }
}

impl PreemptiveResource {
    /// Create a new preemptive resource pool with the given capacity.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is zero.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        assert!(
            capacity > 0,
            "PreemptiveResource capacity must be at least 1"
        );
        PreemptiveResource {
            state: Rc::new(RefCell::new(PreemptiveState {
                wq: WaitQueue::new(capacity),
                holders: Vec::new(),
                next_holder_id: 0,
            })),
        }
    }

    /// Request one unit at the given `priority` (lower = higher priority).
    ///
    /// Resolves immediately if a unit is free **or** if a strictly
    /// lower-priority holder can be preempted; otherwise suspends in priority
    /// order until a unit is released or becomes preemptible.
    ///
    /// The returned [`PreemptiveGuard`] releases the unit when dropped, and
    /// exposes [`preempted`](PreemptiveGuard::preempted) /
    /// [`is_preempted`](PreemptiveGuard::is_preempted) so the holder can yield
    /// the unit cooperatively.
    #[must_use = "futures do nothing unless awaited"]
    pub fn request(&self, priority: u32) -> PreemptiveRequest {
        PreemptiveRequest {
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
        self.state.borrow().wq.in_use()
    }

    /// Total capacity of this resource pool.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.state.borrow().wq.capacity()
    }

    /// Number of processes currently *blocked* waiting for a unit (i.e. those
    /// that could neither take a free unit nor preempt a holder). Excludes
    /// abandoned (canceled) requests and current holders.
    #[must_use]
    pub fn queue_len(&self) -> usize {
        self.state.borrow().wq.live_waiters()
    }
}

/// Build a guard for a freshly granted unit, registering its holder entry.
/// Returns the guard; called from both the free-unit and preemption paths.
fn grant(state_rc: &Rc<RefCell<PreemptiveState>>, priority: u32) -> PreemptiveGuard {
    let (trigger, awaitable) = new_event();
    let preempted = Rc::new(Cell::new(false));
    let id = {
        let mut state = state_rc.borrow_mut();
        let id = state.next_holder_id;
        state.next_holder_id += 1;
        state.holders.push(Holder {
            priority,
            id,
            trigger: Some(trigger),
            preempted: Rc::clone(&preempted),
        });
        id
    };
    PreemptiveGuard {
        state: Rc::clone(state_rc),
        id,
        signal: awaitable,
        preempted,
    }
}

/// Future returned by [`PreemptiveResource::request`].
///
/// Resolves to a [`PreemptiveGuard`] once a unit is acquired — either a free
/// one, or one taken from a preempted lower-priority holder.
pub struct PreemptiveRequest {
    state: Rc<RefCell<PreemptiveState>>,
    priority: u32,
    registered: bool,
    /// Set once a granted/acquired unit has become a holder+guard; `Drop` then
    /// must not release (the guard owns the unit).
    consumed: bool,
    canceled: Rc<Cell<bool>>,
    /// Shared with the queue entry; set to `true` by `WaitQueue::release` when a
    /// released unit is handed directly to this blocked request. Checked first
    /// in `poll`, exactly like the plain `Resource` handoff.
    granted: Rc<Cell<bool>>,
}

impl std::fmt::Debug for PreemptiveRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreemptiveRequest")
            .field("priority", &self.priority)
            .field("registered", &self.registered)
            .field("granted", &self.granted.get())
            .finish_non_exhaustive()
    }
}

impl Future for PreemptiveRequest {
    type Output = PreemptiveGuard;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<PreemptiveGuard> {
        // Direct handoff: a released unit was transferred to us by `wq.release`.
        // `in_use` already accounts for it and the releasing guard removed its
        // holder entry, so we simply register our own holder via grant().
        if self.granted.get() {
            self.consumed = true;
            return Poll::Ready(grant(&self.state, self.priority));
        }

        // Decide the outcome while holding the borrow, but build the guard
        // afterwards (grant() re-borrows state).
        enum Outcome {
            Free,
            Preempt(usize),
            Block,
        }

        let outcome = {
            let mut state = self.state.borrow_mut();
            if state.wq.try_acquire() {
                Outcome::Free
            } else if let Some(idx) = state.victim_index(self.priority) {
                Outcome::Preempt(idx)
            } else {
                if !self.registered {
                    state.wq.register(
                        self.priority,
                        cx.waker().clone(),
                        Rc::clone(&self.canceled),
                        Rc::clone(&self.granted),
                    );
                }
                Outcome::Block
            }
        };

        match outcome {
            Outcome::Free => {
                self.consumed = true;
                Poll::Ready(grant(&self.state, self.priority))
            }
            Outcome::Preempt(idx) => {
                // Evict the victim: fire its signal and remove its holder entry.
                // Capacity bookkeeping is unchanged — the unit transfers
                // directly from victim to us without passing through the queue.
                let victim = {
                    let mut state = self.state.borrow_mut();
                    state.holders.remove(idx)
                };
                victim.preempted.set(true);
                if let Some(trigger) = victim.trigger {
                    trigger.fire();
                }
                self.consumed = true;
                Poll::Ready(grant(&self.state, self.priority))
            }
            Outcome::Block => {
                self.registered = true;
                Poll::Pending
            }
        }
    }
}

impl Drop for PreemptiveRequest {
    fn drop(&mut self) {
        if self.consumed {
            return; // the guard owns the unit and will release it
        }
        if self.granted.get() {
            // A unit was handed to us but never turned into a holder (dropped
            // before re-poll). No holder was created yet, so just release it
            // back into the queue to hand it on to the next waiter.
            self.state.borrow_mut().wq.release();
        } else if self.registered {
            self.canceled.set(true);
        }
    }
}

/// RAII guard holding one unit of a [`PreemptiveResource`].
///
/// Dropping the guard releases the unit (waking the next waiter) **unless** the
/// unit was already preempted, in which case the drop is a no-op — the unit has
/// already been handed to the preemptor.
pub struct PreemptiveGuard {
    state: Rc<RefCell<PreemptiveState>>,
    id: u64,
    signal: EventAwaitable,
    preempted: Rc<Cell<bool>>,
}

impl std::fmt::Debug for PreemptiveGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreemptiveGuard")
            .field("id", &self.id)
            .field("preempted", &self.preempted.get())
            .finish_non_exhaustive()
    }
}

impl PreemptiveGuard {
    /// A future that resolves when this unit is preempted by a higher-priority
    /// request. Intended for racing against the holder's work, e.g.
    /// `any_of![env.timeout(d), guard.preempted()]`.
    ///
    /// Resolves immediately if preemption has already happened.
    #[must_use = "futures do nothing unless awaited"]
    pub fn preempted(&self) -> EventAwaitable {
        self.signal.clone()
    }

    /// Synchronously report whether this unit has been preempted. Check this
    /// after a race to decide whether to abandon the work.
    #[must_use]
    pub fn is_preempted(&self) -> bool {
        self.preempted.get()
    }
}

impl Drop for PreemptiveGuard {
    fn drop(&mut self) {
        let mut state = self.state.borrow_mut();
        // If preempted, the holder entry is already gone and the unit was
        // transferred to the preemptor — releasing again would double-count.
        if self.preempted.get() {
            return;
        }
        if let Some(pos) = state.holders.iter().position(|h| h.id == self.id) {
            state.holders.remove(pos);
        }
        state.wq.release();
    }
}
