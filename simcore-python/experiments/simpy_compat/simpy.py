# SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
#
# SPDX-License-Identifier: MIT OR Apache-2.0

"""A minimal SimPy-compatible API backed by the SimCore kernel.

Drop-in shadow for the SimPy subset that generator-style SimPy models
actually use: put this directory on PYTHONPATH ahead of real simpy and
`import simpy` resolves here.

Covered:
- Environment: now, timeout(delay[, value]), process(generator), run(until)
- Store: put(item), yield store.get(), .items

NOT covered (breaks loudly or behaves differently):
- Event/Process objects with callback lists, interrupts, Process return
  values, AnyOf/AllOf, Resource/PriorityResource/Container (use the simcore
  module's own classes for those), Store capacity (put never blocks),
  env.exit, env.peek/step, simpy.exceptions.

How it works: SimCore's trampoline drives Python generators directly.
`env.timeout()` returns the kernel's awaitable request object; `Store.get()`
returns a plain generator that yields a re-armed kernel Event until an item
is available. Both flow through the same yield-classification path, so a
SimPy process generator runs unchanged.
"""

from collections import deque

import simcore

inf = float("inf")


class Environment:
    """SimPy's Environment, backed by one simcore.Simulation.

    SimPy has no seed (models draw from stdlib `random`); SimCore seeds its
    kernel RNG. Since the kernel RNG is only consumed through simcore's
    explicit rng_* draws, a fixed default seed is fine for SimPy models;
    pass seed=... if the model also uses sim.rng_*.
    """

    def __init__(self, initial_time=0.0, *, seed=0):
        self._sim = simcore.Simulation(seed=seed)
        if initial_time:
            self._sim.run_until(float(initial_time))

    @property
    def now(self):
        return self._sim.now()

    def timeout(self, delay, value=None):
        req = self._sim.timeout(delay)
        if value is None:
            return req  # yieldable kernel request; yield result is None

        def with_value():
            yield req
            return value

        return with_value()

    def process(self, generator):
        # SimPy returns a Process handle; simcore detaches spawned processes,
        # so this returns None. Models relying on the handle (yield proc,
        # proc.interrupt()) are outside the supported subset.
        self._sim.spawn(generator)
        return None

    def run(self, until=None):
        if until is None:
            self._sim.run()
        else:
            # SimPy processes events at exactly `until`; so does
            # simcore's run_until (verified empirically).
            self._sim.run_until(float(until))


class Store:
    """SimPy's Store: FIFO item queue, producers never block.

    Backed by a Python deque plus per-waiter kernel Events: put() wakes
    exactly the oldest get-waiter (matching SimPy, which resumes one
    getter per item). A broadcast-and-recheck design would cost one full
    poll cycle per waiter per item — O(waiters × items) — which
    benchmarks showed to be ~20x slower on contended stores.
    """

    def __init__(self, env, capacity=inf):
        if capacity != inf:
            raise NotImplementedError(
                "simpy_compat.Store does not support bounded capacity "
                "(blocking put); the SimCore kernel has no store primitive"
            )
        self._env = env
        self._items = deque()
        self._waiters = deque()

    @property
    def items(self):
        return self._items

    def put(self, item):
        self._items.append(item)
        if self._waiters:
            self._waiters.popleft().trigger()

    def get(self):
        # A generator (not a coroutine): the trampoline drives it as a
        # sub-routine, and its return value becomes the `yield` result.
        #
        # Race note: get() is called when the user generator is polled and
        # the trampoline primes this sub-generator in the same poll segment,
        # so no put can interleave between creation and the first check.
        # A woken getter whose item was taken by a sibling simply registers
        # a fresh waiter event on the next loop iteration.
        items = self._items
        while not items:
            ev = self._env._sim.event()
            self._waiters.append(ev)
            yield ev
        return items.popleft()
