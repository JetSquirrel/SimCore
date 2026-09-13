# SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
#
# SPDX-License-Identifier: MIT OR Apache-2.0

"""Token-bucket rate limiting with a Container (amount semantics).

A 200k-token bucket throttles agents that each need thousands of tokens per
LLM request. Consumption outpaces the refill, so `get` suspends — the refill
*policy* (how many tokens, how often) lives entirely in a model-layer Python
process; the kernel only provides put/get with strict FIFO.
"""

from simcore import Container, Simulation

CAPACITY = 200_000.0
INIT = 20_000.0
REFILL_AMOUNT = 10_000.0
REFILL_INTERVAL = 1.0


def main() -> None:
    sim = Simulation(seed=42)
    bucket = Container(capacity=CAPACITY, init=INIT)
    pending = [5 * 4]  # model-layer bookkeeping: unfulfilled agent requests

    async def refill() -> None:
        # Policy lives here, in the model: top up periodically while work
        # remains; the kernel only provides FIFO put/get.
        while pending[0] > 0:
            await sim.timeout(REFILL_INTERVAL)
            await bucket.put(REFILL_AMOUNT)
            print(f"t={sim.now():.2f} refill +{REFILL_AMOUNT:.0f} (level={bucket.level():.0f})")

    async def agent(i: int) -> None:
        for r in range(4):
            tokens = 4000.0 + 500.0 * i
            print(f"t={sim.now():.2f} agent {i} request {r} wants {tokens:.0f} tokens (level={bucket.level():.0f})")
            await bucket.get(tokens)
            print(f"t={sim.now():.2f} agent {i} request {r} got tokens (level={bucket.level():.0f})")
            await sim.timeout(0.3)  # LLM call latency
            pending[0] -= 1

    sim.spawn(refill())
    for i in range(5):
        sim.spawn(agent(i))
    sim.run()
    print(f"t={sim.now():.2f} done (level={bucket.level():.0f}, get_queue={bucket.get_queue_len()})")


if __name__ == "__main__":
    main()
