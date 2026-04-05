use std::cell::RefCell;
use std::cmp::Reverse;
use std::future::Future;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll};

use crate::executor::{make_waker, SimState};
use crate::timeout::Timeout;

/// The simulation environment. Central coordinator for a single simulation run.
///
/// `SimEnv` is `!Send + !Sync` (via `Rc`) and must live on one thread.
/// For Monte Carlo parallelism, create independent `SimEnv` instances on
/// separate threads.
pub struct SimEnv {
    state: Rc<RefCell<SimState>>,
}

/// A lightweight handle to the simulation environment, intended to be cloned
/// and passed into spawned processes.
///
/// Both `SimEnv` and `EnvHandle` point to the same underlying `SimState`.
#[derive(Clone)]
pub struct EnvHandle {
    pub(crate) state: Rc<RefCell<SimState>>,
}

impl SimEnv {
    /// Create a new environment starting at time 0.0.
    pub fn new() -> Self {
        SimEnv {
            state: Rc::new(RefCell::new(SimState::new())),
        }
    }

    /// Return a cloneable handle suitable for passing into spawned processes.
    pub fn handle(&self) -> EnvHandle {
        EnvHandle {
            state: Rc::clone(&self.state),
        }
    }

    /// Current simulation time.
    pub fn now(&self) -> f64 {
        self.state.borrow().current_time
    }

    /// Spawn a process. The future is queued and will be polled on the next
    /// executor iteration. May be called before or during `run()`.
    pub fn spawn<F: Future<Output = ()> + 'static>(&self, future: F) {
        let mut state = self.state.borrow_mut();
        let id = state.alloc_process_id();
        state.pending_spawns.push((id, Box::pin(future)));
    }

    /// Create a `Timeout` that resolves after `delay` simulated time units.
    pub fn timeout(&self, delay: f64) -> Timeout {
        self.handle().timeout(delay)
    }

    /// Run until the event queue is empty.
    pub fn run(&mut self) {
        self.drain_pending_spawns();
        self.poll_ready();

        loop {
            let next = self.state.borrow_mut().event_queue.pop();
            match next {
                None => break,
                Some(Reverse(entry)) => {
                    self.state.borrow_mut().current_time = entry.time;
                    entry.waker.wake();
                    self.drain_pending_spawns();
                    self.poll_ready();
                }
            }
        }
    }

    /// Run until simulated time reaches `until`.
    pub fn run_until(&mut self, until: f64) {
        self.drain_pending_spawns();
        self.poll_ready();

        loop {
            let should_stop = {
                let state = self.state.borrow();
                state
                    .event_queue
                    .peek()
                    .map_or(true, |Reverse(e)| e.time > until)
            };

            if should_stop {
                self.state.borrow_mut().current_time = until;
                break;
            }

            let next = self.state.borrow_mut().event_queue.pop();
            if let Some(Reverse(entry)) = next {
                self.state.borrow_mut().current_time = entry.time;
                entry.waker.wake();
                self.drain_pending_spawns();
                self.poll_ready();
            }
        }
    }

    /// Move all pending spawns into the process table and mark them ready.
    fn drain_pending_spawns(&self) {
        let spawns: Vec<_> =
            std::mem::take(&mut self.state.borrow_mut().pending_spawns);
        for (id, fut) in spawns {
            let mut state = self.state.borrow_mut();
            state.processes.insert(id, fut);
            state.ready_queue.lock().unwrap().push(id);
        }
    }

    /// Poll every process in the ready queue until the queue is empty.
    ///
    /// Each process is removed from the process table before polling so that
    /// it can freely borrow `SimState` via its `EnvHandle` without triggering
    /// a `RefCell` panic. Processes that return `Pending` are re-inserted.
    fn poll_ready(&self) {
        loop {
            let ready: Vec<usize> = {
                let state = self.state.borrow();
                let mut rq = state.ready_queue.lock().unwrap();
                std::mem::take(&mut *rq)
            };

            if ready.is_empty() {
                break;
            }

            for id in ready {
                let process = self.state.borrow_mut().processes.remove(&id);

                if let Some(mut process) = process {
                    let ready_queue =
                        Arc::clone(&self.state.borrow().ready_queue);
                    let waker = make_waker(id, ready_queue);
                    let mut cx = Context::from_waker(&waker);

                    if let Poll::Pending = process.as_mut().poll(&mut cx) {
                        self.state.borrow_mut().processes.insert(id, process);
                    }

                    // A process may spawn children during its poll.
                    self.drain_pending_spawns();
                }
            }
        }
    }
}

impl EnvHandle {
    /// Current simulation time.
    pub fn now(&self) -> f64 {
        self.state.borrow().current_time
    }

    /// Create a `Timeout` that resolves after `delay` simulated time units.
    pub fn timeout(&self, delay: f64) -> Timeout {
        let deadline = self.state.borrow().current_time + delay;
        Timeout::new(deadline, self.clone())
    }

    /// Spawn a child process from within a running process.
    pub fn spawn<F: Future<Output = ()> + 'static>(&self, future: F) {
        let mut state = self.state.borrow_mut();
        let id = state.alloc_process_id();
        state.pending_spawns.push((id, Box::pin(future)));
    }
}
