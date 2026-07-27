# SPDX-FileCopyrightText: 2026 Christoph Kuhmuench <christoph.kuhmuench@gmail.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

"""Cross-language known-answer test for the portable feed.

The literals below are the SAME values asserted by the Rust unit tests in
`src/rng.rs` (`KAT_SEED0`, `KAT_SEED42`, and the uniform/exponential answers).
If the Python `_feed.py` and the Rust `simu::rng` ever drift apart, this test
(or its Rust twin) fails.

Run directly: `python compare/models/test_feed.py` (exit 0 on success), or under
pytest if available.
"""

from _feed import SplitMix64

KAT_SEED0 = [
    0xE220A8397B1DCDAF,
    0x6E789E6AA1B965F4,
    0x06C45D188009454F,
    0xF88BB8A8724C81EC,
    0x1B39896A51A8749B,
    0x53CB9F0C747EA2EA,
]
KAT_SEED42 = [
    0xBDD732262FEB6E95,
    0x28EFE333B266F103,
    0x47526757130F9F52,
    0x581CE1FF0E4AE394,
    0x09BC585A244823F2,
    0xDE4431FA3C80DB06,
]


def test_splitmix64_known_answer_vectors():
    for seed, expected in [(0, KAT_SEED0), (42, KAT_SEED42)]:
        rng = SplitMix64(seed)
        got = [rng.next_u64() for _ in expected]
        assert got == expected, f"seed {seed}: {got}"


def test_uniform01_and_exponential_known_answer():
    assert SplitMix64(0).uniform01() == 0.8833108082136426
    assert SplitMix64(0).exponential(1.0) == 2.148241359348383


def test_reseed_restarts_stream():
    rng = SplitMix64(0)
    first = rng.next_u64()
    rng.next_u64()
    rng.set_seed(0)
    assert rng.next_u64() == first


if __name__ == "__main__":
    test_splitmix64_known_answer_vectors()
    test_uniform01_and_exponential_known_answer()
    test_reseed_restarts_stream()
    print("ok: feed known-answer vectors match Rust")
