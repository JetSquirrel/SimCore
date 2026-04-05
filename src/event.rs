use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

struct EventState {
    fired: bool,
    waiters: Vec<Waker>,
}

/// The sending half of a manual event. Call [`fire`](EventTrigger::fire) to
/// wake all processes currently waiting on the paired [`EventAwaitable`], and
/// to make any *future* awaits on that same awaitable resolve immediately.
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
#[derive(Clone)]
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
