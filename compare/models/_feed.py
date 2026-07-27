# SPDX-FileCopyrightText: 2026 Christoph Kuhmuench <christoph.kuhmuench@gmail.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

"""Portable random feed — the Python twin of `simu::rng` (src/rng.rs).

This is a byte-for-byte re-implementation of the SplitMix64 generator and the
closed-form sampling transforms that the Rust side uses (`SplitMix64` plus
`simu::rng::sample`). Seeded with the same value, a Rust run and a Python run
draw the *same* `u64` stream and turn it into the *same* samples, so the harness
can compare per-seed metrics exactly rather than only in distribution.

Keep this in lockstep with `src/rng.rs`. `test_feed.py` asserts a shared
known-answer table that is also checked by the Rust unit tests; if either side
drifts, one of the two test suites fails.
"""

import math

MASK64 = (1 << 64) - 1
# 2^-53, matching the Rust `TWO_POW_NEG_53` constant.
INV_2_POW_53 = 1.0 / 9007199254740992.0


class SplitMix64:
    """Deterministic SplitMix64 PRNG with shared sampling transforms."""

    __slots__ = ("state",)

    def __init__(self, seed):
        self.state = seed & MASK64

    def set_seed(self, seed):
        """Reset the feed, restarting the stream from the beginning."""
        self.state = seed & MASK64

    def next_u64(self):
        self.state = (self.state + 0x9E3779B97F4A7C15) & MASK64
        z = self.state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK64
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK64
        return z ^ (z >> 31)

    def uniform01(self):
        """Uniform float in [0, 1) from the top 53 bits of a u64."""
        return (self.next_u64() >> 11) * INV_2_POW_53

    def exponential(self, mean):
        """Exponential variate with the given mean (inverse-CDF)."""
        u = self.uniform01()
        return -mean * math.log1p(-u)

    def bernoulli(self, p):
        """True with probability p, consuming exactly one uniform."""
        return self.uniform01() < p

    def normal(self, mean, std):
        """Normal variate via Box-Muller, consuming exactly two uniforms."""
        u1 = self.uniform01()
        u2 = self.uniform01()
        r = math.sqrt(-2.0 * math.log1p(-u1))
        z = r * math.cos(math.tau * u2)
        return mean + std * z
