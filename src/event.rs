// SPDX-FileCopyrightText: 2026 Christoph Kuhmuench <christoph.kuhmuench@gmail.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

#[derive(Debug)]
struct EventState {
    fired: bool,
    waiters: Vec<Waker>,
}

/// The sending half of a manual event. Call [`fire`](EventTrigger::fire) to
/// wake all processes currently waiting on the paired [`EventAwaitable`], and
/// to make any *future* awaits on that same awaitable resolve immediately.
///
/// **Dropping a trigger without firing it strands its waiters.** If an
/// `EventTrigger` is dropped without `fire()` being called, the event never
/// fires: any process currently awaiting the paired `EventAwaitable` — and any
/// that awaits it later — will suspend forever (until the run ends and
/// [`SimEnv`](crate::SimEnv)'s `Drop` reclaims the suspended processes). This is
/// a normal discrete-event outcome (a signal that simply never arrives), not a
/// panic; if a process must not block indefinitely, race the awaitable against a
/// [`timeout`](crate::EnvHandle::timeout) via [`any_of!`](crate::any_of).
#[derive(Debug)]
pub struct EventTrigger {
    state: Rc<RefCell<EventState>>,
}

/// The receiving half of a manual event.
///
/// Implements `Future<Output = ()>`. If the paired [`EventTrigger`] has
/// already fired, polling returns `Ready` immediately. Otherwise the calling
/// process is suspended and woken when [`EventTrigger::fire`] is called.
///
/// `EventAwaitable` is `Clone`: every clone shares the same underlying event,
/// so multiple processes can await the same trigger.
#[derive(Clone, Debug)]
pub struct EventAwaitable {
    state: Rc<RefCell<EventState>>,
}

/// Create a paired `(EventTrigger, EventAwaitable)`.
pub(crate) fn new_event() -> (EventTrigger, EventAwaitable) {
    let state = Rc::new(RefCell::new(EventState {
        fired: false,
        waiters: Vec::new(),
    }));
    (
        EventTrigger { state: Rc::clone(&state) },
        EventAwaitable { state },
    )
}

impl EventTrigger {
    /// Fire the event.
    ///
    /// All processes currently suspended on the paired `EventAwaitable` are
    /// woken immediately. Any process that awaits the event *after* this call
    /// will also resolve without suspending.
    pub fn fire(self) {
        let mut state = self.state.borrow_mut();
        state.fired = true;
        for waker in state.waiters.drain(..) {
            waker.wake();
        }
    }
}

impl Future for EventAwaitable {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut state = self.state.borrow_mut();
        if state.fired {
            return Poll::Ready(());
        }
        // Register this waker only if not already present (avoid duplicates on
        // repeated polls from the same task).
        let waker = cx.waker();
        if !state.waiters.iter().any(|w| w.will_wake(waker)) {
            state.waiters.push(waker.clone());
        }
        Poll::Pending
    }
}
