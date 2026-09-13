# SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
#
# SPDX-License-Identifier: MIT OR Apache-2.0

"""Binding demo: run_until boundaries, manual events, sub-generator stack.

Covers the three SimPy-compatibility extensions (fresh Simulation per
section so the traces stay crisp):
- run_until is boundary-EXCLUSIVE (an event scheduled at exactly `until`
  does not run, yet now() reports `until`) and the simulation stays
  resumable afterwards — the pending boundary event fires on the next run.
- Simulation.event() / Event.trigger(): one-shot latch, multiple waiters.
- spawn() accepts plain generators; `yield sub()` drives sub-generators and
  delivers their return value as the yield result.
"""

from simcore import Simulation


def sleeper(sim, name, delay):
    yield sim.timeout(delay)
    print(f"t={sim.now():.2f} {name} woke")
    return f"{name}-result"


def run_until_demo() -> None:
    print("== run_until ==")
    sim = Simulation(seed=1)
    sim.spawn(sleeper(sim, "at-boundary", 5.0))
    sim.spawn(sleeper(sim, "past-boundary", 6.0))
    sim.run_until(5.0)
    print(f"t={sim.now():.2f} after run_until(5.0)")
    sim.run_until(10.0)
    print(f"t={sim.now():.2f} after resume run_until(10.0)")


def event_demo() -> None:
    print("== manual event ==")
    sim = Simulation(seed=1)
    ev = sim.event()

    def waiter(i):
        yield ev
        print(f"t={sim.now():.2f} waiter {i} released (triggered={ev.triggered})")

    def trigger_later():
        yield sim.timeout(2.0)
        ev.trigger()
        try:
            ev.trigger()
        except RuntimeError as e:
            print(f"t={sim.now():.2f} second trigger -> RuntimeError: {e}")

    def late():
        yield sim.timeout(3.0)  # event already fired by then: no extra wait
        yield ev
        print(f"t={sim.now():.2f} late waiter passed immediately")

    sim.spawn(waiter(0))
    sim.spawn(waiter(1))
    sim.spawn(trigger_later())
    sim.spawn(late())
    sim.run()


def subgenerator_demo() -> None:
    print("== sub-generators ==")
    sim = Simulation(seed=1)

    def inner(depth):
        yield sim.timeout(0.5)
        if depth > 0:
            sub = yield inner(depth - 1)
            return f"inner({depth} got {sub})"
        return "bottom"

    def outer():
        result = yield inner(2)
        print(f"t={sim.now():.2f} chain returned: {result}")

    sim.spawn(outer())
    sim.run()


def subgenerator_error_demo() -> None:
    print("== sub-generator error ==")
    sim = Simulation(seed=2)

    def boom_child():
        yield sim.timeout(0.1)
        raise ValueError("sub-generator exploded")

    def boom_parent():
        yield boom_child()

    sim.spawn(boom_parent())
    try:
        sim.run()
        print("FAIL: no error")
    except ValueError as e:
        print(f"run() re-raised from sub-generator: {e}")


if __name__ == "__main__":
    run_until_demo()
    event_demo()
    subgenerator_demo()
    subgenerator_error_demo()
