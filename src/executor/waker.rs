// SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::{Arc, Mutex};
use std::task::Wake;

/// Waker for a simulation process. When woken, pushes the process ID onto
/// the shared ready queue so the executor will poll it on the next iteration.
pub(crate) struct SimWaker {
    pub process_id: usize,
    pub ready_queue: Arc<Mutex<Vec<usize>>>,
}

impl Wake for SimWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.ready_queue
            .lock()
            .expect("simu ready-queue mutex poisoned")
            .push(self.process_id);
    }
}

pub(crate) fn make_waker(
    process_id: usize,
    ready_queue: Arc<Mutex<Vec<usize>>>,
) -> std::task::Waker {
    std::task::Waker::from(Arc::new(SimWaker {
        process_id,
        ready_queue,
    }))
}
