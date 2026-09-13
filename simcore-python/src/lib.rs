// SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! PyO3 bindings for the `simcore` DES kernel.
//!
//! Architecture under test: the Rust kernel executor steps Python coroutines
//! directly (trampoline pattern) with no asyncio involvement. Each spawned
//! Python coroutine is wrapped in exactly one Rust process; the wrapper calls
//! `coro.send(...)`, classifies the request object the coroutine's awaitable
//! yielded, awaits the matching kernel future, then resumes the coroutine.
//!
//! Protocol notes (verified empirically against CPython 3.14.6):
//! - `await x` runs `yield from x.__await__()`; the sub-iterator is primed
//!   with `next()`. Non-None values sent into the coroutine are delegated to
//!   the sub-iterator's `send(v)` (PEP 380); a sent None goes to
//!   `tp_iternext` (`__next__`) instead. When the sub-iterator raises
//!   `StopIteration(v)`, `v` becomes the await result.
//! - So the trampoline delivers each await result by sending it into the
//!   coroutine; `Driver.send(v)` raises `StopIteration(v)`. A `__next__`
//!   fallback (raising `StopIteration(fallback)`) covers the None/`next()`
//!   path: fallback is None for timeouts, the request itself for acquires.
//! - `async with r.acquire() as x:` awaits `__aenter__()`; the trampoline
//!   sends the AcquireRequest back, so `x` binds the request (which holds
//!   the kernel guard until `__aexit__` drops it).

use std::cell::RefCell;
use std::rc::Rc;

use pyo3::exceptions::{PyRuntimeError, PyStopIteration, PyTypeError, PyValueError};
use pyo3::prelude::*;

use ::simcore::rng::sample;
use ::simcore::{
    Container as KernelContainer, EnvHandle, EventAwaitable, EventTrigger,
    PriorityResource as KernelPriorityResource, PriorityResourceGuard,
    Resource as KernelResource, ResourceGuard, SimEnv, SplitMix64,
};

/// The simulation environment: owns the kernel `SimEnv` and collects the first
/// process error so `run()` can re-raise it with a real traceback.
#[pyclass(unsendable)]
struct Simulation {
    /// Taken out during `run()` so the pyclass borrow is never held across the
    /// event loop (processes call back into `now`/`timeout`/`spawn` mid-run).
    env: RefCell<Option<SimEnv>>,
    handle: EnvHandle,
    error: Rc<RefCell<Option<PyErr>>>,
}

/// What a stepped coroutine yielded, classified for the trampoline.
enum Request {
    Timeout(f64),
    Acquire(Py<AcquireRequest>),
    PriorityAcquire(Py<PriorityAcquireRequest>),
    ContainerGet(Py<ContainerGetRequest>),
    ContainerPut(Py<ContainerPutRequest>),
    Event(EventAwaitable),
    /// A plain Python generator driven as a sub-routine (SimPy-style
    /// `yield sub_process()` composition).
    Subroutine(Py<PyAny>),
}

#[pymethods]
impl Simulation {
    /// `portable=True` drives the env from the portable SplitMix64 feed
    /// instead of rand's StdRng — same seed then gives the same draw stream
    /// as the pure-Python feed mirrored in the kernel's comparison harness.
    #[new]
    #[pyo3(signature = (seed, *, portable = false))]
    fn new(seed: u64, portable: bool) -> Self {
        let env = if portable {
            SimEnv::with_source(SplitMix64::new(seed))
        } else {
            SimEnv::with_seed(seed)
        };
        let handle = env.handle();
        Simulation {
            env: RefCell::new(Some(env)),
            handle,
            error: Rc::new(RefCell::new(None)),
        }
    }

    /// Current simulation time.
    fn now(&self) -> f64 {
        self.handle.now()
    }

    /// Build an awaitable that suspends the process for `t` simulated units.
    fn timeout(&self, t: f64) -> PyResult<TimeoutRequest> {
        if !(t >= 0.0 && t.is_finite()) {
            return Err(PyValueError::new_err(format!(
                "timeout delay must be finite and non-negative (got {t})"
            )));
        }
        Ok(TimeoutRequest { t })
    }

    /// Draw a uniform f64 in [0, 1) from the kernel RNG stream (the same
    /// stream Rust processes draw from via `EnvHandle::rng()`).
    ///
    /// Safe to call inside processes (mid-run) and outside `run()`: the
    /// handle's shared RNG cell is only borrowed for the duration of the
    /// draw, never across an await.
    fn rng_random(&self) -> f64 {
        sample::uniform01(&mut self.handle.rng())
    }

    /// Draw a uniform f64 in [a, b]. Consumes one draw.
    fn rng_uniform(&self, a: f64, b: f64) -> PyResult<f64> {
        if !(a.is_finite() && b.is_finite() && a <= b) {
            return Err(PyValueError::new_err(format!(
                "rng_uniform requires finite bounds with a <= b (got a={a}, b={b})"
            )));
        }
        Ok(a + (b - a) * self.rng_random())
    }

    /// Draw an integer uniformly from [a, b] inclusive. Consumes one draw.
    fn rng_range_int(&self, a: i64, b: i64) -> PyResult<i64> {
        if a > b {
            return Err(PyValueError::new_err(format!(
                "rng_range_int requires a <= b (got a={a}, b={b})"
            )));
        }
        let span = (b - a) as f64 + 1.0;
        Ok(a + (self.rng_random() * span).floor() as i64)
    }

    /// Draw an exponential variate with the given mean (inverse-CDF, the
    /// transform mirrored in the kernel's cross-language harness).
    fn rng_exponential(&self, mean: f64) -> PyResult<f64> {
        if !(mean > 0.0 && mean.is_finite()) {
            return Err(PyValueError::new_err(format!(
                "rng_exponential mean must be positive and finite (got {mean})"
            )));
        }
        Ok(sample::exponential(&mut self.handle.rng(), mean))
    }

    /// Wrap a Python coroutine *or generator* in one Rust process driven by
    /// the kernel. Yielded generators are driven as sub-routines on an
    /// explicit stack (SimPy-style composition); yielded simcore request
    /// objects are classified and awaited as kernel futures.
    fn spawn(&self, process: Py<PyAny>) {
        let handle = self.handle.clone();
        let error = Rc::clone(&self.error);
        self.handle.spawn(async move {
            let mut stack: Vec<Py<PyAny>> = vec![process];
            // Value sent into the top-of-stack generator on the next step;
            // the first send into a fresh generator must be None.
            let mut resume: Option<Py<PyAny>> = None;
            while !stack.is_empty() {
                let top = Python::attach(|py| stack.last().unwrap().clone_ref(py));
                let step = Python::attach(|py| {
                    let value = match &resume {
                        Some(v) => v.clone_ref(py),
                        None => py.None(),
                    };
                    top.call_method1(py, "send", (value,))
                });
                let yielded = match step {
                    Ok(y) => y,
                    Err(e) => {
                        let stop_value = Python::attach(|py| {
                            if e.is_instance_of::<PyStopIteration>(py) {
                                // The generator's return value, delivered to
                                // the parent frame as the `yield` result.
                                Some(e.value(py).getattr("value").unwrap().unbind())
                            } else {
                                None
                            }
                        });
                        match stop_value {
                            Some(v) => {
                                stack.pop();
                                resume = Some(v);
                                continue;
                            }
                            // A real exception anywhere in the stack
                            // terminates the whole process.
                            None => {
                                *error.borrow_mut() = Some(e);
                                break;
                            }
                        }
                    }
                };
                let request = Python::attach(|py| -> PyResult<Request> {
                    let obj = yielded.bind(py);
                    if let Ok(t) = obj.cast::<TimeoutRequest>() {
                        Ok(Request::Timeout(t.borrow().t))
                    } else if let Ok(a) = obj.cast::<AcquireRequest>() {
                        Ok(Request::Acquire(a.clone().unbind()))
                    } else if let Ok(a) = obj.cast::<PriorityAcquireRequest>() {
                        Ok(Request::PriorityAcquire(a.clone().unbind()))
                    } else if let Ok(g) = obj.cast::<ContainerGetRequest>() {
                        Ok(Request::ContainerGet(g.clone().unbind()))
                    } else if let Ok(p) = obj.cast::<ContainerPutRequest>() {
                        Ok(Request::ContainerPut(p.clone().unbind()))
                    } else if let Ok(ev) = obj.cast::<Event>() {
                        Ok(Request::Event(ev.borrow().awaitable.clone()))
                    } else if obj.hasattr("gi_frame")? {
                        // A Python generator (gi_frame is generator-specific;
                        // pyo3's PyGenerator type is unavailable under abi3):
                        // drive it as a sub-routine.
                        Ok(Request::Subroutine(yielded.clone_ref(py)))
                    } else {
                        let ty = obj.get_type().name()?;
                        Err(PyTypeError::new_err(format!(
                            "simcore processes may only yield generators or simcore \
                             request objects (timeout, acquire, container get/put, event), \
                             got object of type '{ty}'"
                        )))
                    }
                });
                match request {
                    Ok(Request::Timeout(t)) => {
                        handle.timeout(t).await;
                        resume = None; // await sim.timeout(..) evaluates to None
                    }
                    Ok(Request::Acquire(acquire)) => {
                        let resource =
                            Python::attach(|py| acquire.borrow(py).resource.borrow(py).inner.clone());
                        let guard = resource.request().await;
                        Python::attach(|py| acquire.borrow_mut(py).guard = Some(guard));
                        // await result = the AcquireRequest itself (holds the guard)
                        resume = Some(acquire.into_any());
                    }
                    Ok(Request::PriorityAcquire(acquire)) => {
                        let (resource, priority) = Python::attach(|py| {
                            let req = acquire.borrow(py);
                            let resource = req.resource.borrow(py).inner.clone();
                            (resource, req.priority)
                        });
                        let guard = resource.request(priority).await;
                        Python::attach(|py| acquire.borrow_mut(py).guard = Some(guard));
                        resume = Some(acquire.into_any());
                    }
                    Ok(Request::ContainerGet(get)) => {
                        let (container, amount) = Python::attach(|py| {
                            let req = get.borrow(py);
                            let container = req.container.borrow(py).inner.clone();
                            (container, req.amount)
                        });
                        container.get(amount).await; // kernel future resolves to ()
                        resume = None;
                    }
                    Ok(Request::ContainerPut(put)) => {
                        let (container, amount) = Python::attach(|py| {
                            let req = put.borrow(py);
                            let container = req.container.borrow(py).inner.clone();
                            (container, req.amount)
                        });
                        container.put(amount).await;
                        resume = None;
                    }
                    Ok(Request::Event(awaitable)) => {
                        awaitable.await;
                        resume = None;
                    }
                    Ok(Request::Subroutine(gen)) => {
                        stack.push(gen);
                        resume = None; // prime the sub-generator
                    }
                    Err(e) => {
                        *error.borrow_mut() = Some(e);
                        break;
                    }
                }
            }
        });
    }

    /// Create a manual event: `.trigger()` fires it once; yielding the event
    /// (or awaiting it) suspends until fired. Fire-before-await is
    /// remembered (latch semantics), matching the kernel.
    fn event(&self) -> Event {
        let (trigger, awaitable) = self.handle.event();
        Event {
            trigger: Some(trigger),
            awaitable,
        }
    }

    /// Run the simulation until the event queue drains; re-raise the first
    /// process error (if any) so Python callers see real tracebacks.
    ///
    /// The `SimEnv` is taken out of the cell for the duration so no pyclass
    /// or field borrow is held while processes call back into this object.
    fn run(&self) -> PyResult<()> {
        let mut env = self.take_env()?;
        env.run();
        *self.env.borrow_mut() = Some(env);
        self.check_error()
    }

    /// Run until simulated time reaches `until` (exclusive: an event
    /// scheduled at exactly `until` is not run), then stop with the clock at
    /// `until`. The simulation stays resumable — a later `run()` /
    /// `run_until()` picks up where this one stopped.
    fn run_until(&self, until: f64) -> PyResult<()> {
        if !(until.is_finite() && until >= 0.0) {
            return Err(PyValueError::new_err(format!(
                "run_until boundary must be finite and non-negative (got {until})"
            )));
        }
        let mut env = self.take_env()?;
        env.run_until(until);
        *self.env.borrow_mut() = Some(env);
        self.check_error()
    }
}

impl Simulation {
    fn take_env(&self) -> PyResult<SimEnv> {
        self.env
            .borrow_mut()
            .take()
            .ok_or_else(|| PyRuntimeError::new_err("Simulation is already running"))
    }

    fn check_error(&self) -> PyResult<()> {
        if let Some(e) = self.error.borrow_mut().take() {
            return Err(e);
        }
        Ok(())
    }
}

/// A capacity-limited FIFO resource pool (single-unit acquire).
#[pyclass(unsendable)]
struct Resource {
    inner: KernelResource,
}

#[pymethods]
impl Resource {
    #[new]
    fn new(capacity: usize) -> PyResult<Self> {
        if capacity == 0 {
            return Err(PyValueError::new_err("Resource capacity must be at least 1"));
        }
        Ok(Resource {
            inner: KernelResource::new(capacity),
        })
    }

    /// Request one unit; await it or use `async with`. Single-unit only —
    /// the kernel Resource has no amount semantics.
    fn acquire(slf: Py<Self>) -> AcquireRequest {
        AcquireRequest {
            resource: slf,
            guard: None,
        }
    }

    fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    fn in_use(&self) -> usize {
        self.inner.in_use()
    }

    fn queue_len(&self) -> usize {
        self.inner.queue_len()
    }
}

/// A capacity-limited priority-scheduled resource pool (single-unit acquire;
/// lower priority number = served first, FIFO within a level).
#[pyclass(unsendable)]
struct PriorityResource {
    inner: KernelPriorityResource,
}

#[pymethods]
impl PriorityResource {
    #[new]
    fn new(capacity: usize) -> PyResult<Self> {
        if capacity == 0 {
            return Err(PyValueError::new_err(
                "PriorityResource capacity must be at least 1",
            ));
        }
        Ok(PriorityResource {
            inner: KernelPriorityResource::new(capacity),
        })
    }

    /// Request one unit at `priority` (lower = higher priority); await it or
    /// use `async with`.
    fn acquire(slf: Py<Self>, priority: u32) -> PriorityAcquireRequest {
        PriorityAcquireRequest {
            resource: slf,
            priority,
            guard: None,
        }
    }

    fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    fn in_use(&self) -> usize {
        self.inner.in_use()
    }

    fn queue_len(&self) -> usize {
        self.inner.queue_len()
    }
}

/// A continuous-quantity reservoir (token bucket, tank, inventory). `get`
/// suspends until enough material is present, `put` until enough space is
/// free; both are strict head-of-line FIFO. There is no guard: a `get` is a
/// one-shot withdrawal, so put-back-on-exit policy belongs to the model.
#[pyclass(unsendable)]
struct Container {
    inner: KernelContainer,
}

/// Validate a get/put amount the way the kernel asserts, as a Python error.
fn check_amount(op: &str, amount: f64, capacity: f64) -> PyResult<()> {
    if !(amount.is_finite() && amount > 0.0) {
        return Err(PyValueError::new_err(format!(
            "Container::{op} amount must be positive and finite (got {amount})"
        )));
    }
    if amount > capacity {
        return Err(PyValueError::new_err(format!(
            "Container::{op} amount ({amount}) exceeds capacity ({capacity}); it could never complete"
        )));
    }
    Ok(())
}

#[pymethods]
impl Container {
    #[new]
    #[pyo3(signature = (capacity, init = 0.0))]
    fn new(capacity: f64, init: f64) -> PyResult<Self> {
        if !(capacity.is_finite() && capacity > 0.0) {
            return Err(PyValueError::new_err(format!(
                "Container capacity must be positive and finite (got {capacity})"
            )));
        }
        if !(init.is_finite() && (0.0..=capacity).contains(&init)) {
            return Err(PyValueError::new_err(format!(
                "Container init must be in [0, capacity] (got init={init}, capacity={capacity})"
            )));
        }
        Ok(Container {
            inner: KernelContainer::new(capacity, init),
        })
    }

    /// Withdraw `amount`; awaitable, suspends until the level covers it.
    fn get(slf: Py<Self>, py: Python<'_>, amount: f64) -> PyResult<ContainerGetRequest> {
        let capacity = slf.borrow(py).inner.capacity();
        check_amount("get", amount, capacity)?;
        Ok(ContainerGetRequest {
            container: slf,
            amount,
        })
    }

    /// Add `amount`; awaitable, suspends until enough space is free.
    fn put(slf: Py<Self>, py: Python<'_>, amount: f64) -> PyResult<ContainerPutRequest> {
        let capacity = slf.borrow(py).inner.capacity();
        check_amount("put", amount, capacity)?;
        Ok(ContainerPutRequest {
            container: slf,
            amount,
        })
    }

    fn level(&self) -> f64 {
        self.inner.level()
    }

    fn capacity(&self) -> f64 {
        self.inner.capacity()
    }

    fn get_queue_len(&self) -> usize {
        self.inner.get_queue_len()
    }

    fn put_queue_len(&self) -> usize {
        self.inner.put_queue_len()
    }
}

/// Awaitable priority acquire request; mirrors [`AcquireRequest`].
#[pyclass(unsendable)]
struct PriorityAcquireRequest {
    resource: Py<PriorityResource>,
    priority: u32,
    guard: Option<PriorityResourceGuard>,
}

#[pymethods]
impl PriorityAcquireRequest {
    fn __await__(slf: Py<Self>, py: Python<'_>) -> Driver {
        let fallback = slf.clone_ref(py).into_any();
        Driver::new(slf.into_any(), fallback)
    }

    fn __aenter__(slf: Py<Self>, py: Python<'_>) -> Driver {
        let fallback = slf.clone_ref(py).into_any();
        Driver::new(slf.into_any(), fallback)
    }

    /// Drop the guard (releasing the unit, waking the highest-priority
    /// waiter) synchronously, then complete immediately with False.
    fn __aexit__(
        &mut self,
        _exc_type: &Bound<'_, PyAny>,
        _exc: &Bound<'_, PyAny>,
        _tb: &Bound<'_, PyAny>,
    ) -> Completed {
        self.guard = None;
        Completed
    }

    /// Explicit release for the bare `await res.acquire(prio)` style.
    fn release(&mut self) {
        self.guard = None;
    }

    #[getter]
    fn priority(&self) -> u32 {
        self.priority
    }

    #[getter]
    fn acquired(&self) -> bool {
        self.guard.is_some()
    }
}

/// Awaitable container `get(amount)`; resolves to None once the level was
/// deducted.
#[pyclass]
struct ContainerGetRequest {
    container: Py<Container>,
    amount: f64,
}

#[pymethods]
impl ContainerGetRequest {
    fn __await__(slf: Py<Self>, py: Python<'_>) -> Driver {
        Driver::new(slf.into_any(), py.None())
    }

    #[getter]
    fn amount(&self) -> f64 {
        self.amount
    }
}

/// Awaitable container `put(amount)`; resolves to None once the amount was
/// added.
#[pyclass]
struct ContainerPutRequest {
    container: Py<Container>,
    amount: f64,
}

#[pymethods]
impl ContainerPutRequest {
    fn __await__(slf: Py<Self>, py: Python<'_>) -> Driver {
        Driver::new(slf.into_any(), py.None())
    }

    #[getter]
    fn amount(&self) -> f64 {
        self.amount
    }
}

/// A one-shot manual event (SimPy-style signalling). `.trigger()` fires it,
/// waking all current waiters; any later await resolves immediately (latch).
/// Yielding the event itself from a process suspends until fired.
#[pyclass(unsendable)]
struct Event {
    trigger: Option<EventTrigger>,
    awaitable: EventAwaitable,
}

#[pymethods]
impl Event {
    /// Fire the event. One-shot: a second call raises RuntimeError (the
    /// kernel trigger is consumed by firing).
    fn trigger(&mut self) -> PyResult<()> {
        match self.trigger.take() {
            Some(t) => {
                t.fire();
                Ok(())
            }
            None => Err(PyRuntimeError::new_err("Event was already triggered")),
        }
    }

    /// Whether the event has fired yet.
    #[getter]
    fn triggered(&self) -> bool {
        self.trigger.is_none()
    }

    fn __await__(slf: Py<Self>, py: Python<'_>) -> Driver {
        Driver::new(slf.into_any(), py.None())
    }
}

/// Awaitable timeout request; the trampoline reads `t` and awaits the kernel.
#[pyclass]
struct TimeoutRequest {
    t: f64,
}

#[pymethods]
impl TimeoutRequest {
    fn __await__(slf: Py<Self>, py: Python<'_>) -> Driver {
        Driver::new(slf.into_any(), py.None())
    }
}

/// Awaitable acquire request. Holds the kernel `ResourceGuard` once the
/// trampoline has acquired the unit, until `__aexit__`/`release()` drops it.
#[pyclass(unsendable)]
struct AcquireRequest {
    resource: Py<Resource>,
    guard: Option<ResourceGuard>,
}

#[pymethods]
impl AcquireRequest {
    fn __await__(slf: Py<Self>, py: Python<'_>) -> Driver {
        let fallback = slf.clone_ref(py).into_any();
        Driver::new(slf.into_any(), fallback)
    }

    /// `async with` enter: awaiting this Driver suspends until the trampoline
    /// stores the guard here; the await result is this request object.
    fn __aenter__(slf: Py<Self>, py: Python<'_>) -> Driver {
        let fallback = slf.clone_ref(py).into_any();
        Driver::new(slf.into_any(), fallback)
    }

    /// `async with` exit: drop the guard (releasing the unit and waking the
    /// FIFO next waiter) synchronously, then complete immediately with False.
    fn __aexit__(
        &mut self,
        _exc_type: &Bound<'_, PyAny>,
        _exc: &Bound<'_, PyAny>,
        _tb: &Bound<'_, PyAny>,
    ) -> Completed {
        self.guard = None;
        Completed
    }

    /// Explicit release for the bare `await res.acquire()` style.
    fn release(&mut self) {
        self.guard = None;
    }

    #[getter]
    fn acquired(&self) -> bool {
        self.guard.is_some()
    }
}

/// Iterator returned by `__await__`: yields the request object exactly once,
/// then completes. `send(v)` raises `StopIteration(v)` (the await result is
/// whatever the trampoline sent); the plain `next()` fallback raises
/// `StopIteration(fallback)`, which is None for timeouts and the request
/// itself for acquires.
#[pyclass]
struct Driver {
    request: Py<PyAny>,
    fallback: Py<PyAny>,
    yielded: bool,
}

impl Driver {
    fn new(request: Py<PyAny>, fallback: Py<PyAny>) -> Self {
        Driver {
            request,
            fallback,
            yielded: false,
        }
    }
}

#[pymethods]
impl Driver {
    fn __iter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    /// Lets `async with` await the Driver returned by `__aenter__` directly.
    fn __await__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    fn __next__(&mut self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        if self.yielded {
            // Plain `next()` delivers no result value; use the fallback.
            Err(PyStopIteration::new_err((self.fallback.clone_ref(py),)))
        } else {
            self.yielded = true;
            Ok(self.request.clone_ref(py))
        }
    }

    /// PEP 380 delegation: non-None values sent into the coroutine land
    /// here; raising `StopIteration(v)` makes the `await` expression
    /// evaluate to `v`. (None is never delegated to `send` — CPython uses
    /// `tp_iternext` for it — but handle it via the fallback anyway.)
    fn send(&mut self, py: Python<'_>, value: Py<PyAny>) -> PyResult<Py<PyAny>> {
        if self.yielded {
            let result = if value.is_none(py) {
                self.fallback.clone_ref(py)
            } else {
                value
            };
            Err(PyStopIteration::new_err((result,)))
        } else {
            self.yielded = true;
            Ok(self.request.clone_ref(py))
        }
    }
}

/// Awaitable returned by `__aexit__`: completes immediately with False.
#[pyclass]
struct Completed;

#[pymethods]
impl Completed {
    fn __await__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    fn __iter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    fn __next__(&self) -> PyResult<()> {
        Err(PyStopIteration::new_err((false,)))
    }
}

#[pymodule]
fn simcore(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Simulation>()?;
    m.add_class::<Resource>()?;
    m.add_class::<PriorityResource>()?;
    m.add_class::<Container>()?;
    m.add_class::<TimeoutRequest>()?;
    m.add_class::<AcquireRequest>()?;
    m.add_class::<PriorityAcquireRequest>()?;
    m.add_class::<ContainerGetRequest>()?;
    m.add_class::<ContainerPutRequest>()?;
    m.add_class::<Event>()?;
    m.add_class::<Driver>()?;
    m.add_class::<Completed>()?;
    Ok(())
}
