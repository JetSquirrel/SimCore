mod queue;
mod waker;

pub(crate) use queue::ScheduledWaker;
pub(crate) use waker::make_waker;

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

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
    pub processes: HashMap<usize, Pin<Box<dyn Future<Output = ()>>>>,
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
            processes: HashMap::new(),
            pending_spawns: Vec::new(),
        }
    }

    /// Enqueue a wakeup at `time`. The monotonically increasing `seq_counter`
    /// ensures that events scheduled at the same time are processed in
    /// insertion order, keeping simulation output deterministic.
    pub fn schedule_wakeup(&mut self, time: f64, waker: std::task::Waker) {
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
