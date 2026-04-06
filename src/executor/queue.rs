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

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::{RawWaker, RawWakerVTable, Waker};

    fn noop_waker() -> Waker {
        const VTABLE: RawWakerVTable =
            RawWakerVTable::new(|p| RawWaker::new(p, &VTABLE), |_| {}, |_| {}, |_| {});
        unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
    }

    fn make(time: f64, seq: u64) -> ScheduledWaker {
        ScheduledWaker { time, seq, waker: noop_waker() }
    }

    #[test]
    fn partial_eq_same_time_and_seq() {
        assert!(make(1.0, 0) == make(1.0, 0));
    }

    #[test]
    fn partial_eq_different_seq() {
        assert!(make(1.0, 0) != make(1.0, 1));
    }

    #[test]
    fn partial_eq_different_time() {
        assert!(make(1.0, 0) != make(2.0, 0));
    }
}
