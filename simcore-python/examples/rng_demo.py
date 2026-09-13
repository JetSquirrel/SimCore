# SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
#
# SPDX-License-Identifier: MIT OR Apache-2.0

"""RNG determinism and parity demo.

Draws come from the kernel's seeded RNG stream (the same stream Rust
processes use). Two checks:
1. Same seed -> identical draw sequence across runs (diff this output).
2. portable=True uses the SplitMix64 feed; its first uniform01 draw for
   seed 0 is the kernel's documented known-answer value (asserted in the
   kernel's own test suite), proving the binding drives the real stream.
"""

from simcore import Simulation

SPLITMIX64_SEED0_FIRST_UNIFORM01 = 0.8833108082136426  # kernel KAT


def main() -> None:
    sim = Simulation(seed=42)
    print("== rng draws (StdRng, seed=42) ==")
    print("rng_random:", [f"{sim.rng_random():.6f}" for _ in range(5)])
    print("rng_uniform(2, 5):", [f"{sim.rng_uniform(2.0, 5.0):.6f}" for _ in range(3)])
    print("rng_range_int(1, 6):", [sim.rng_range_int(1, 6) for _ in range(8)])
    print("rng_exponential(10):", [f"{sim.rng_exponential(10.0):.6f}" for _ in range(3)])

    # Draws from inside a process come from the same stream.
    async def worker() -> None:
        print(f"t={sim.now():.2f} in-process draw: {sim.rng_random():.6f}")

    print(f"pre-run draw:  {sim.rng_random():.6f}")
    sim.spawn(worker())
    sim.run()
    print(f"post-run draw: {sim.rng_random():.6f}")

    portable = Simulation(seed=0, portable=True)
    first = portable.rng_random()
    print(f"portable SplitMix64 seed=0 first draw: {first}")
    assert first == SPLITMIX64_SEED0_FIRST_UNIFORM01, "parity with kernel KAT broken"
    print("parity with kernel known-answer value: OK")


if __name__ == "__main__":
    main()
