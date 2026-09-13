# SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
#
# SPDX-License-Identifier: MIT OR Apache-2.0

"""any_of / all_of demo, including loser-cancellation checks per arm type.

For every supported arm type we race a request that cannot complete in time
against a short timeout, let the timeout win, and then verify the kernel
state is clean (unit not held, level unchanged, waiter deregistered) and
the SAME Python request object can be awaited again successfully.
"""

from simcore import Container, PriorityResource, Resource, Simulation


def main() -> None:
    sim = Simulation(seed=7)

    # --- any_of: event beats timeout (timeout arm is the loser) ----------
    ev = sim.event()

    async def fire_later() -> None:
        await sim.timeout(2.0)
        ev.trigger()

    async def arm_event() -> None:
        sim.spawn(fire_later())
        race = sim.any_of(sim.timeout(5.0), ev)
        winner = await race
        print(f"t={sim.now():.2f} any_of(timeout=5, event@2) -> winner={winner} (race.winner={race.winner})")
        # event as the LOSING arm: timeout wins, dropped signal arm is harmless
        never = sim.event()
        winner = await sim.any_of(sim.timeout(0.5), never)
        print(f"t={sim.now():.2f} event lost race -> winner={winner}, triggered={never.triggered}")

    sim.spawn(arm_event())
    sim.run_until(3.0)

    # --- any_of: resource acquire loses, kernel state stays clean --------
    pump = Resource(capacity=1)

    async def arm_acquire() -> None:
        holder = await pump.acquire()  # occupy the only unit
        req = pump.acquire()  # would block until the holder releases
        winner = await sim.any_of(req, sim.timeout(1.0))
        print(f"t={sim.now():.2f} acquire lost race -> winner={winner}, acquired={req.acquired}, "
              f"in_use={pump.in_use()}, queue={pump.queue_len()}")
        holder.release()
        req2 = await req  # the losing request object is still usable
        print(f"t={sim.now():.2f} re-awaited losing acquire -> acquired={req2.acquired}")
        req2.release()

    sim.spawn(arm_acquire())
    sim.run_until(6.0)

    # --- any_of: priority acquire loses -----------------------------------
    prio = PriorityResource(capacity=1)

    async def arm_priority() -> None:
        holder = await prio.acquire(priority=0)

        async def waiter() -> None:
            req = prio.acquire(priority=5)
            winner = await sim.any_of(req, sim.timeout(1.0))
            print(f"t={sim.now():.2f} priority acquire lost -> winner={winner}, acquired={req.acquired}, "
                  f"queue={prio.queue_len()}")
            holder.release()
            req2 = await req
            print(f"t={sim.now():.2f} re-awaited losing priority acquire -> acquired={req2.acquired}, "
                  f"priority={req2.priority}")
            req2.release()

        sim.spawn(waiter())

    sim.spawn(arm_priority())
    sim.run_until(9.0)

    # --- any_of: container get/put lose, levels unchanged -----------------
    tank = Container(capacity=100.0, init=10.0)

    async def arm_container() -> None:
        get_req = tank.get(50.0)  # cannot complete at level 10
        winner = await sim.any_of(get_req, sim.timeout(1.0))
        print(f"t={sim.now():.2f} container get lost -> winner={winner}, level={tank.level():.0f}, "
              f"get_queue={tank.get_queue_len()}")
        await tank.put(90.0)
        await get_req  # re-await the same request object
        print(f"t={sim.now():.2f} re-awaited losing get -> level={tank.level():.0f}")

        full = Container(capacity=10.0, init=10.0)
        put_req = full.put(5.0)  # cannot complete: no space
        winner = await sim.any_of(put_req, sim.timeout(1.0))
        print(f"t={sim.now():.2f} container put lost -> winner={winner}, level={full.level():.0f}, "
              f"put_queue={full.put_queue_len()}")

    sim.spawn(arm_container())
    sim.run_until(14.0)

    # --- all_of: resolves when the slowest arm completes ------------------
    async def arm_all() -> None:
        await sim.all_of(sim.timeout(1.0), sim.timeout(3.0), sim.timeout(5.0))
        print(f"t={sim.now():.2f} all_of(1, 3, 5) resolved")

        r = Resource(capacity=2)
        a, b = r.acquire(), r.acquire()
        await sim.all_of(a, b)
        print(f"t={sim.now():.2f} all_of(acquire, acquire) -> a={a.acquired}, b={b.acquired}, "
              f"in_use={r.in_use()}")
        a.release()
        b.release()

    sim.spawn(arm_all())
    sim.run()
    print(f"t={sim.now():.2f} done")


if __name__ == "__main__":
    main()
