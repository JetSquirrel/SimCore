use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::env::EnvHandle;

/// A `Future` that resolves once simulated time reaches `deadline`.
///
/// On first poll it registers a wakeup in the event queue and returns
/// `Pending`. The executor wakes it when the event fires, and the next
/// poll returns `Ready`.
pub struct Timeout {
    deadline: f64,
    scheduled: bool,
    env: EnvHandle,
}

impl Timeout {
    pub(crate) fn new(deadline: f64, env: EnvHandle) -> Self {
        Timeout {
            deadline,
            scheduled: false,
            env,
        }
    }
}

impl Future for Timeout {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.scheduled {
            Poll::Ready(())
        } else {
            self.env.schedule_wakeup(self.deadline, cx.waker().clone());
            self.scheduled = true;
            Poll::Pending
        }
    }
}
