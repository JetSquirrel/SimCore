# SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
#
# SPDX-License-Identifier: MIT OR Apache-2.0

"""PreemptiveResource demo: premium evicts a running bulk call.

Capacity-1 preemptive pool. bulk-0 (prio 10) starts a 10 s call at t=0,
racing its service against the preemption signal. premium (prio 0) arrives
at t=2 and evicts it; bulk-0 discovers the preemption at its next await
(cooperative-at-yield) and abandons. bulk-1 (also prio 10) was queued and
gets the unit when premium releases.
"""

from simcore import PreemptiveResource, Simulation


def main() -> None:
    sim = Simulation(seed=42)
    gpu = PreemptiveResource(capacity=1)

    async def bulk(i: int) -> None:
        async with gpu.acquire(priority=10) as slot:
            print(f"t={sim.now():.2f} bulk {i} started a 10.0s call")
            winner = await sim.any_of(sim.timeout(10.0), slot.preempt_event())
            if slot.preempted:
                print(f"t={sim.now():.2f} bulk {i} PREEMPTED (arm {winner}) — abandoning call")
            else:
                print(f"t={sim.now():.2f} bulk {i} completed")

    async def premium() -> None:
        await sim.timeout(2.0)
        print(f"t={sim.now():.2f} premium arrives (queue={gpu.queue_len()}, in_use={gpu.in_use()})")
        async with gpu.acquire(priority=0):
            print(f"t={sim.now():.2f} premium ACQUIRED by eviction")
            await sim.timeout(1.0)
        print(f"t={sim.now():.2f} premium done")

    sim.spawn(bulk(0))
    sim.spawn(bulk(1))
    sim.spawn(premium())
    sim.run()
    print(f"t={sim.now():.2f} done (in_use={gpu.in_use()}, queue={gpu.queue_len()})")


if __name__ == "__main__":
    main()
