use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

// ---------------------------------------------------------------------------
// AnyOf — resolves when the first sub-future resolves
// ---------------------------------------------------------------------------

/// A future that resolves when **any one** of its sub-futures resolves.
///
/// All sub-futures must have `Output = ()`, which is the common output type
/// of all simu event primitives (`Timeout`, `EventAwaitable`, etc.).
///
/// When a sub-future resolves, the remaining ones are dropped. Any wakers they
/// registered may still fire later; the executor handles such spurious wakeups
/// gracefully.
///
/// Prefer the [`any_of!`](crate::any_of) macro over constructing this directly.
pub struct AnyOf {
    futures: Vec<Pin<Box<dyn Future<Output = ()>>>>,
}

impl std::fmt::Debug for AnyOf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnyOf")
            .field("pending", &self.futures.len())
            .finish()
    }
}

impl AnyOf {
    /// Create an `AnyOf` combinator from a list of futures.
    ///
    /// # Panics
    ///
    /// Panics if `futures` is empty — waiting for "any of nothing" is a logic
    /// error.
    #[must_use = "futures do nothing unless awaited"]
    pub fn new(futures: Vec<Pin<Box<dyn Future<Output = ()>>>>) -> Self {
        assert!(!futures.is_empty(), "AnyOf requires at least one future");
        AnyOf { futures }
    }
}

impl Future for AnyOf {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        for fut in self.get_mut().futures.iter_mut() {
            if fut.as_mut().poll(cx).is_ready() {
                return Poll::Ready(());
            }
        }
        Poll::Pending
    }
}

// ---------------------------------------------------------------------------
// AllOf — resolves when all sub-futures have resolved
// ---------------------------------------------------------------------------

/// A future that resolves when **all** of its sub-futures have resolved.
///
/// All sub-futures must have `Output = ()`. Completed sub-futures are dropped
/// eagerly so they are not polled again after returning `Ready`.
///
/// Resolves immediately if constructed with an empty list (vacuously true).
///
/// Prefer the [`all_of!`](crate::all_of) macro over constructing this directly.
pub struct AllOf {
    futures: Vec<Pin<Box<dyn Future<Output = ()>>>>,
}

impl std::fmt::Debug for AllOf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AllOf")
            .field("pending", &self.futures.len())
            .finish()
    }
}

impl AllOf {
    /// Create an `AllOf` combinator from a list of futures.
    ///
    /// Resolves immediately if `futures` is empty.
    #[must_use = "futures do nothing unless awaited"]
    pub fn new(futures: Vec<Pin<Box<dyn Future<Output = ()>>>>) -> Self {
        AllOf { futures }
    }
}

impl Future for AllOf {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // Retain only futures that are still pending; completed ones are dropped.
        // Both AnyOf and AllOf are Unpin (Vec and Pin<Box<...>> are Unpin),
        // so get_mut() is safe here.
        let this = self.as_mut().get_mut();
        this.futures
            .retain_mut(|fut| fut.as_mut().poll(cx).is_pending());

        if this.futures.is_empty() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}

// ---------------------------------------------------------------------------
// Convenience macros
// ---------------------------------------------------------------------------

/// Wait for the **first** of several futures to resolve.
///
/// Each expression is automatically pinned in a `Box`. All futures must have
/// `Output = ()`.
///
/// # Example
///
/// ```
/// use simu::{SimEnv, any_of};
/// let mut env = SimEnv::with_seed(0);
/// let h = env.handle();
/// let (trigger, signal) = env.event();
/// env.spawn(async move { h.timeout(3.0).await; trigger.fire(); });
/// let h2 = env.handle();
/// env.spawn(async move {
///     // resolves at t=3 (the event) rather than t=10 (the timeout)
///     any_of![h2.timeout(10.0), signal.clone()].await;
/// });
/// env.run();
/// ```
#[macro_export]
macro_rules! any_of {
    ($($fut:expr),+ $(,)?) => {
        $crate::AnyOf::new(
            vec![$(::std::boxed::Box::pin($fut)),+]
        )
    };
}

/// Wait for **all** of several futures to resolve.
///
/// Each expression is automatically pinned in a `Box`. All futures must have
/// `Output = ()`.
///
/// # Example
///
/// ```
/// use simu::{SimEnv, all_of};
/// let mut env = SimEnv::with_seed(0);
/// let h = env.handle();
/// env.spawn(async move {
///     // resolves at t=5, when the slowest sub-future completes
///     all_of![h.timeout(1.0), h.timeout(3.0), h.timeout(5.0)].await;
/// });
/// env.run();
/// ```
#[macro_export]
macro_rules! all_of {
    ($($fut:expr),+ $(,)?) => {
        $crate::AllOf::new(
            vec![$(::std::boxed::Box::pin($fut)),+]
        )
    };
}
