use std::cell::RefCell;
use std::cmp::Reverse;
use std::future::Future;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll};

use rand::rngs::StdRng;
use rand::{RngCore, SeedableRng};

use crate::event::{new_event, EventAwaitable, EventTrigger};
use crate::executor::{make_waker, SimState};
use crate::timeout::Timeout;

/// The simulation environment. Central coordinator for a single simulation run.
///
/// `SimEnv` is `!Send + !Sync` (via `Rc`) and must live on one thread.
/// For Monte Carlo parallelism, create independent `SimEnv` instances on
/// separate threads.
pub struct SimEnv {
    state: Rc<RefCell<SimState>>,
    rng: Rc<RefCell<StdRng>>,
}

/// A lightweight handle to the simulation environment, intended to be cloned
/// and passed into spawned processes.
///
/// Both `SimEnv` and all `EnvHandle` clones share the same underlying
/// `SimState` and the same `StdRng` instance.
#[derive(Clone)]
pub struct EnvHandle {
    pub(crate) state: Rc<RefCell<SimState>>,
    pub(crate) rng: Rc<RefCell<StdRng>>,
}

impl SimEnv {
    /// Create a new environment seeded from OS entropy.
    ///
    /// Use [`with_seed`](SimEnv::with_seed) when reproducibility is required.
    pub fn new() -> Self {
        SimEnv {
            state: Rc::new(RefCell::new(SimState::new())),
            rng: Rc::new(RefCell::new(StdRng::from_entropy())),
        }
    }

    /// Create a new environment with a fixed RNG seed.
    ///
    /// Given the same seed and process logic the simulation will produce
    /// identical results across runs.
    pub fn with_seed(seed: u64) -> Self {
        SimEnv {
            state: Rc::new(RefCell::new(SimState::new())),
            rng: Rc::new(RefCell::new(StdRng::seed_from_u64(seed))),
        }
    }

    /// Return a cloneable handle suitable for passing into spawned processes.
    pub fn handle(&self) -> EnvHandle {
        EnvHandle {
            state: Rc::clone(&self.state),
            rng: Rc::clone(&self.rng),
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

    /// Create a paired `(EventTrigger, EventAwaitable)` for inter-process signalling.
    pub fn event(&self) -> (EventTrigger, EventAwaitable) {
        new_event()
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

    /// Borrow the shared RNG mutably.
    ///
    /// The returned guard derefs to `StdRng`, which implements `rand::Rng`,
    /// so distributions can be sampled directly:
    ///
    /// ```ignore
    /// let duration = env.rng().sample(Exp::new(1.0 / 20.0).unwrap());
    /// ```
    pub fn rng(&self) -> impl rand::RngCore + '_ {
        RngGuard(self.rng.borrow_mut())
    }

    /// Create a `Timeout` that resolves after `delay` simulated time units.
    pub fn timeout(&self, delay: f64) -> Timeout {
        let deadline = self.state.borrow().current_time + delay;
        Timeout::new(deadline, self.clone())
    }

    /// Create a paired `(EventTrigger, EventAwaitable)` for inter-process signalling.
    pub fn event(&self) -> (EventTrigger, EventAwaitable) {
        new_event()
    }

    /// Spawn a child process from within a running process.
    pub fn spawn<F: Future<Output = ()> + 'static>(&self, future: F) {
        let mut state = self.state.borrow_mut();
        let id = state.alloc_process_id();
        state.pending_spawns.push((id, Box::pin(future)));
    }
}

/// Newtype wrapper so `EnvHandle::rng()` can return an `impl RngCore + '_`
/// without exposing `RefMut` in the public API.
struct RngGuard<'a>(std::cell::RefMut<'a, StdRng>);

impl RngCore for RngGuard<'_> {
    fn next_u32(&mut self) -> u32 { self.0.next_u32() }
    fn next_u64(&mut self) -> u64 { self.0.next_u64() }
    fn fill_bytes(&mut self, dest: &mut [u8]) { self.0.fill_bytes(dest) }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand::Error> {
        self.0.try_fill_bytes(dest)
    }
}
