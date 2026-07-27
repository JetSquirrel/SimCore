// SPDX-FileCopyrightText: 2026 Christoph Kuhmuench <christoph.kuhmuench@gmail.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Observable spawned processes.
//!
//! [`SimEnv::spawn`](crate::env::SimEnv::spawn) and
//! [`EnvHandle::spawn`](crate::env::EnvHandle::spawn) return a
//! [`ProcessHandle<T>`] that implements `Future<Output = T>`. Awaiting the
//! handle suspends the calling process until the spawned process finishes
//! and yields its return value.
//!
//! The handle is **not `Clone`** — a single awaiter per process, matching
//! `tokio::JoinHandle`. Broadcast/completion-signal patterns already have
//! [`EventTrigger`](crate::EventTrigger).

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

/// Shared slot between the spawn-wrapper and the handle.
struct ProcessSlot<T> {
    result: Option<T>,
    waker:  Option<Waker>,
}

/// Handle to a spawned process. Resolves to the process's return value.
///
/// Dropping the handle before awaiting detaches the process — it continues to
/// run; its return value, if any, is dropped when the process completes.
///
/// ```
/// use simu::SimEnv;
///
/// let mut env = SimEnv::with_seed(0);
/// let h = env.handle();
/// env.spawn(async move {
///     let hc = h.clone();
///     let child = h.spawn(async move {
///         hc.timeout(3.0).await;
///         "charged" // the child's return value
///     });
///     let result = child.await; // suspend until the child finishes
///     assert_eq!(result, "charged");
///     assert_eq!(h.now(), 3.0);
/// });
/// env.run();
/// ```
pub struct ProcessHandle<T> {
    slot: Rc<RefCell<ProcessSlot<T>>>,
}

impl<T> std::fmt::Debug for ProcessHandle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("ProcessHandle");
        if let Ok(slot) = self.slot.try_borrow() {
            d.field("ready", &slot.result.is_some());
        }
        d.finish_non_exhaustive()
    }
}

impl<T: 'static> Future for ProcessHandle<T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        let mut slot = self.slot.borrow_mut();
        if let Some(value) = slot.result.take() {
            return Poll::Ready(value);
        }
        // Dedup the waker — avoid cloning on every re-poll from the same task.
        let waker = cx.waker();
        match &slot.waker {
            Some(existing) if existing.will_wake(waker) => {}
            _ => slot.waker = Some(waker.clone()),
        }
        Poll::Pending
    }
}

impl<T: 'static> ProcessHandle<T> {
    /// Await the handle and discard the return value.
    ///
    /// Useful with [`any_of!`](crate::any_of) / [`all_of!`](crate::all_of),
    /// whose sub-futures must have `Output = ()`.
    pub async fn discard(self) {
        let _ = self.await;
    }
}

/// Build the `(wrapper future, handle)` pair used by `EnvHandle::spawn`.
///
/// The wrapper type-erases `F::Output` into `()` so the process table can stay
/// `HashMap<_, Pin<Box<dyn Future<Output = ()>>>>`. When the user's future
/// resolves, the wrapper stores the value in the shared slot and wakes the
/// handle's awaiter (if any).
#[allow(clippy::type_complexity)]
pub(crate) fn spawn_with_handle<F>(
    future: F,
) -> (Pin<Box<dyn Future<Output = ()>>>, ProcessHandle<F::Output>)
where
    F: Future + 'static,
    F::Output: 'static,
{
    let slot = Rc::new(RefCell::new(ProcessSlot {
        result: None,
        waker:  None,
    }));
    let slot_for_wrapper = Rc::clone(&slot);

    let wrapped = async move {
        let result = future.await;
        let mut s = slot_for_wrapper.borrow_mut();
        s.result = Some(result);
        if let Some(w) = s.waker.take() {
            w.wake();
        }
    };

    (Box::pin(wrapped), ProcessHandle { slot })
}
