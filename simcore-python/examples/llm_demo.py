# SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
#
# SPDX-License-Identifier: MIT OR Apache-2.0

"""Demo: Python coroutines driven by the simcore Rust kernel (no asyncio).

Three agents contend for a capacity-2 "Claude API" resource. Run twice and
diff the output: it must be byte-identical (deterministic event ordering).
"""

from simcore import Resource, Simulation


def async_with_demo() -> None:
    print("== async with demo ==")
    sim = Simulation(seed=42)
    claude = Resource(capacity=2)

    async def agent(i: int) -> None:
        for r in range(3):
            print(f"t={sim.now():.2f} agent {i} request {r} waiting")
            async with claude.acquire() as req:
                print(f"t={sim.now():.2f} agent {i} request {r} acquired (in_use={claude.in_use()}, acquired={req.acquired})")
                await sim.timeout(0.8 + 0.1 * i)
            print(f"t={sim.now():.2f} agent {i} done request {r}")

    for i in range(3):
        sim.spawn(agent(i))
    sim.run()
    print(f"t={sim.now():.2f} simulation finished")


def bare_acquire_demo() -> None:
    print("== bare acquire demo ==")
    sim = Simulation(seed=7)
    pump = Resource(capacity=1)

    async def worker(i: int) -> None:
        req = await pump.acquire()
        print(f"t={sim.now():.2f} worker {i} acquired (queue={pump.queue_len()})")
        await sim.timeout(1.0)
        req.release()
        print(f"t={sim.now():.2f} worker {i} released")

    for i in range(2):
        sim.spawn(worker(i))
    sim.run()
    print(f"t={sim.now():.2f} simulation finished")


if __name__ == "__main__":
    async_with_demo()
    bare_acquire_demo()
