# SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
#
# SPDX-License-Identifier: MIT OR Apache-2.0

"""Standalone demo of the SimPy shim (no YAFS dependency).

Run with the shim directory on PYTHONPATH ahead of real simpy:
    PYTHONPATH=experiments/simpy_compat python experiments/simpy_compat/demo.py
Exercises Environment/timeout/process/run(until)/Store with multiple
producers and consumers, including a consumer that waits twice.
"""

import simpy  # resolves to the shim when PYTHONPATH is set


def main() -> None:
    env = simpy.Environment()
    store = simpy.Store(env)

    def producer(i):
        for r in range(2):
            yield env.timeout(1.0 + 0.5 * i)
            item = f"msg-{i}.{r}"
            print(f"t={env.now:.2f} producer {i} put {item}")
            store.put(item)

    def consumer(i):
        for _ in range(2):
            item = yield store.get()
            print(f"t={env.now:.2f} consumer {i} got {item}")

    def clock():
        # Runs past the boundary: events at exactly until must still fire.
        yield env.timeout(3.0)
        print(f"t={env.now:.2f} boundary event fired")

    env.process(producer(0))
    env.process(producer(1))
    env.process(consumer(0))
    env.process(consumer(1))
    env.process(clock())
    env.run(until=3.0)
    print(f"t={env.now:.2f} stopped at run(until=3.0)")
    env.run()  # resume to completion
    print(f"t={env.now:.2f} done, leftover items: {list(store.items)}")


if __name__ == "__main__":
    main()
