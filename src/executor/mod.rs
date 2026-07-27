// SPDX-FileCopyrightText: 2026 Christoph Kuhmuench <christoph.kuhmuench@gmail.com>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

mod queue;
mod waker;

pub(crate) use queue::ScheduledWaker;
pub(crate) use waker::make_waker;

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::Waker;

/// A live process: its suspended future plus the single `Waker` cached for it.
///
/// The waker is created once when the process is admitted and reused on every
/// poll, so (a) we avoid an `Arc` allocation per poll and (b) waker de-dup
/// (`Waker::will_wake`) in `EventAwaitable` / `ProcessHandle` actually works —
/// every poll of a given process presents the *same* waker.
pub(crate) struct ProcessEntry {
    pub future: Pin<Box<dyn Future<Output = ()>>>,
    pub waker: Waker,
}

/// All mutable simulation state shared between `SimEnv` and `EnvHandle`.
///
/// Stored behind `Rc<RefCell<SimState>>`. The `ready_queue` is additionally
/// wrapped in `Arc<Mutex<>>` so it can be referenced from `Waker` vtables,
/// which require `Send + Sync`. In practice the mutex is never contended
/// because the simulation is single-threaded.
pub(crate) struct SimState {
    pub current_time: f64,
    pub event_queue: BinaryHeap<Reverse<ScheduledWaker>>,
    pub seq_counter: u64,
    pub ready_queue: Arc<Mutex<Vec<usize>>>,
    pub next_process_id: usize,
    /// Process table indexed by the dense, monotonic process id (never reused).
    /// A slot is `Some` while its process is live and `None` once it completes;
    /// completed processes leave a `None` gap for the rest of the run (cheap:
    /// one pointer-sized slot each), which is fine for simulation lifetimes and
    /// gives O(1), hash-free take/put-back on the hot poll path.
    pub processes: Vec<Option<ProcessEntry>>,
    #[allow(clippy::type_complexity)]
    pub pending_spawns: Vec<(usize, Pin<Box<dyn Future<Output = ()>>>)>,
}

impl SimState {
    pub fn new() -> Self {
        SimState {
            current_time: 0.0,
            event_queue: BinaryHeap::new(),
            seq_counter: 0,
            ready_queue: Arc::new(Mutex::new(Vec::new())),
            next_process_id: 0,
            processes: Vec::new(),
            pending_spawns: Vec::new(),
        }
    }

    /// Enqueue a wakeup at `time`. The monotonically increasing `seq_counter`
    /// ensures that events scheduled at the same time are processed in
    /// insertion order, keeping simulation output deterministic.
    pub fn schedule_wakeup(&mut self, time: f64, waker: std::task::Waker) {
        // Belt-and-braces: public entry points (e.g. `EnvHandle::timeout`) reject
        // non-finite or past deadlines, so a scheduled wakeup must never rewind
        // the clock. A violation here means an internal caller bypassed that guard.
        debug_assert!(
            time.is_finite() && time >= self.current_time,
            "schedule_wakeup time must be finite and not in the past"
        );
        let seq = self.seq_counter;
        self.seq_counter += 1;
        self.event_queue.push(Reverse(ScheduledWaker { time, seq, waker }));
    }

    pub fn alloc_process_id(&mut self) -> usize {
        let id = self.next_process_id;
        self.next_process_id += 1;
        id
    }
}
