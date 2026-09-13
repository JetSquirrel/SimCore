# SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
#
# SPDX-License-Identifier: MIT OR Apache-2.0

"""Priority scheduling demo: premium agents overtake bulk agents.

One capacity-1 PriorityResource. A holder occupies it until t=2. Three bulk
agents (priority 10) queue at t=0; two premium agents (priority 0) arrive
later, at t=1 — yet when the holder releases, premium is served first
(lower number = higher priority; FIFO within a level).
"""

from simcore import PriorityResource, Simulation


def main() -> None:
    sim = Simulation(seed=42)
    gpu = PriorityResource(capacity=1)

    async def holder() -> None:
        async with gpu.acquire(priority=5):
            print(f"t={sim.now():.2f} holder occupied the GPU until t=2.00")
            await sim.timeout(2.0)

    async def tenant(kind: str, i: int, priority: int, arrive: float) -> None:
        await sim.timeout(arrive)
        print(f"t={sim.now():.2f} {kind} {i} (prio {priority}) queued (queue={gpu.queue_len()})")
        async with gpu.acquire(priority=priority):
            print(f"t={sim.now():.2f} {kind} {i} (prio {priority}) ACQUIRED")
            await sim.timeout(0.5)

    sim.spawn(holder())
    for i in range(3):
        sim.spawn(tenant("bulk", i, 10, 0.0))
    for j in range(2):
        sim.spawn(tenant("premium", j, 0, 1.0))
    sim.run()
    print(f"t={sim.now():.2f} done")


if __name__ == "__main__":
    main()
