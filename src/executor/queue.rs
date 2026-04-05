use std::cmp::Ordering;
use std::task::Waker;

/// An entry in the simulation event queue.
///
/// Ordered by (time, seq) ascending — used with `Reverse` in a `BinaryHeap`
/// to form a min-heap. The sequence number breaks ties deterministically.
pub(crate) struct ScheduledWaker {
    pub time: f64,
    pub seq: u64,
    pub waker: Waker,
}

impl PartialEq for ScheduledWaker {
    fn eq(&self, other: &Self) -> bool {
        self.time == other.time && self.seq == other.seq
    }
}

impl Eq for ScheduledWaker {}

impl PartialOrd for ScheduledWaker {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ScheduledWaker {
    fn cmp(&self, other: &Self) -> Ordering {
        self.time
            .partial_cmp(&other.time)
            .unwrap_or(Ordering::Equal)
            .then(self.seq.cmp(&other.seq))
    }
}
