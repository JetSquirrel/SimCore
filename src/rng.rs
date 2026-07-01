//! Pluggable randomness sources and a portable, deterministic feed.
//!
//! By default a [`SimEnv`](crate::SimEnv) draws from `rand`'s `StdRng`. For
//! cross-language reproducibility (e.g. the SimPy comparison harness) the env
//! can instead be driven from any [`RandomSource`] via
//! [`SimEnv::with_source`](crate::SimEnv::with_source).
//!
//! This module ships [`SplitMix64`], a tiny portable PRNG whose output stream
//! is defined purely by integer arithmetic, so it can be re-implemented
//! byte-for-byte in another language (see `compare/models/_feed.py`). Combined
//! with the closed-form transforms in [`sample`], two engines seeded with the
//! same value draw the *same* numbers and turn them into the *same* samples.
//!
//! The two layers compose: every [`sample`] function takes any `RngCore`, so it
//! works over `StdRng` or a [`SplitMix64`] feed alike.

use rand::rngs::StdRng;
use rand::{RngCore, SeedableRng};

/// A pluggable source of randomness for a [`SimEnv`](crate::SimEnv).
///
/// The `RngCore` supertrait means `env.rng().sample(dist)` keeps working for
/// every source. Implementations that are seed-based override [`reseed`] so
/// that [`SimEnv::set_seed`](crate::SimEnv::set_seed) can restart their stream;
/// sources that are not seed-based (e.g. a recorded-data feed) may keep the
/// default, which panics — consistent with the crate's "programming error ⇒
/// panic" policy.
///
/// [`reseed`]: RandomSource::reseed
pub trait RandomSource: RngCore {
    /// Re-seed the source deterministically, restarting its stream.
    ///
    /// The default implementation panics. Override it for seed-based sources.
    fn reseed(&mut self, seed: u64) {
        let _ = seed;
        panic!("this random source does not support reseeding");
    }
}

impl RandomSource for StdRng {
    fn reseed(&mut self, seed: u64) {
        *self = StdRng::seed_from_u64(seed);
    }
}

/// A portable, deterministic PRNG (SplitMix64).
///
/// SplitMix64 is a well-known generator whose output is defined entirely by
/// wrapping `u64` arithmetic with fixed constants, so it can be re-implemented
/// identically in any language. `simu` uses it as the shared "external feed"
/// that lets a Rust run and a Python run draw the same number stream from the
/// same seed.
///
/// It is fast and has good statistical quality for simulation, but it is *not*
/// cryptographically secure — do not use it where unpredictability matters.
#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Create a feed seeded with `seed`.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        SplitMix64 { state: seed }
    }

    /// Reset the feed to `seed`, restarting the stream from the beginning.
    pub fn set_seed(&mut self, seed: u64) {
        self.state = seed;
    }

    /// Advance the state and return the next 64-bit output.
    #[inline]
    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

impl RngCore for SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.next()
    }

    fn next_u32(&mut self) -> u32 {
        // Take the high 32 bits — mirrored exactly on the Python side.
        (self.next() >> 32) as u32
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        let mut chunks = dest.chunks_exact_mut(8);
        for chunk in &mut chunks {
            chunk.copy_from_slice(&self.next().to_le_bytes());
        }
        let rem = chunks.into_remainder();
        if !rem.is_empty() {
            let bytes = self.next().to_le_bytes();
            rem.copy_from_slice(&bytes[..rem.len()]);
        }
    }
}

impl RandomSource for SplitMix64 {
    fn reseed(&mut self, seed: u64) {
        self.state = seed;
    }
}

/// Closed-form sampling transforms shared with the Python comparison harness.
///
/// Each function consumes raw output from any `RngCore` and applies a transform
/// that is mirrored *exactly* in `compare/models/_feed.py`. Driving both engines
/// from a [`SplitMix64`] feed plus these transforms makes their per-draw samples
/// agree to floating-point tolerance.
///
/// ```
/// use simu::rng::{sample, SplitMix64};
/// let mut feed = SplitMix64::new(42);
/// let u = sample::uniform01(&mut feed); // in [0, 1)
/// assert!((0.0..1.0).contains(&u));
/// ```
pub mod sample {
    use rand::RngCore;

    /// 2^-53, used to map a 53-bit integer into `[0, 1)`.
    const TWO_POW_NEG_53: f64 = 1.0 / 9_007_199_254_740_992.0;

    /// Draw a uniform `f64` in `[0, 1)` using the top 53 bits of a `u64`.
    ///
    /// Matches NumPy's `random_double` construction so the value is identical
    /// to the Python feed.
    pub fn uniform01<R: RngCore + ?Sized>(rng: &mut R) -> f64 {
        ((rng.next_u64() >> 11) as f64) * TWO_POW_NEG_53
    }

    /// Draw an exponential variate with the given `mean` via inverse-CDF.
    ///
    /// `-mean * ln(1 - u)`, computed with `ln_1p(-u)` for accuracy.
    pub fn exponential<R: RngCore + ?Sized>(rng: &mut R, mean: f64) -> f64 {
        let u = uniform01(rng);
        -mean * (-u).ln_1p()
    }

    /// Draw a Bernoulli trial that is `true` with probability `p`.
    pub fn bernoulli<R: RngCore + ?Sized>(rng: &mut R, p: f64) -> bool {
        uniform01(rng) < p
    }

    /// Draw a normal variate via Box–Muller, consuming exactly two uniforms.
    ///
    /// Only the cosine arm is used (no caching of the sine arm) so the draw
    /// count per call is fixed and matches the Python feed.
    pub fn normal<R: RngCore + ?Sized>(rng: &mut R, mean: f64, std: f64) -> f64 {
        let u1 = uniform01(rng);
        let u2 = uniform01(rng);
        let r = (-2.0 * (-u1).ln_1p()).sqrt();
        let z = r * (std::f64::consts::TAU * u2).cos();
        mean + std * z
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shared cross-language known-answer table. The same literals are asserted
    // in `compare/models/test_feed.py`; if either implementation drifts, one of
    // the two test suites fails. Values are the canonical SplitMix64 outputs.
    const KAT_SEED0: [u64; 6] = [
        0xE220_A839_7B1D_CDAF,
        0x6E78_9E6A_A1B9_65F4,
        0x06C4_5D18_8009_454F,
        0xF88B_B8A8_724C_81EC,
        0x1B39_896A_51A8_749B,
        0x53CB_9F0C_747E_A2EA,
    ];
    const KAT_SEED42: [u64; 6] = [
        0xBDD7_3226_2FEB_6E95,
        0x28EF_E333_B266_F103,
        0x4752_6757_130F_9F52,
        0x581C_E1FF_0E4A_E394,
        0x09BC_585A_2448_23F2,
        0xDE44_31FA_3C80_DB06,
    ];

    #[test]
    fn splitmix64_known_answer_vectors() {
        for (seed, expected) in [(0u64, KAT_SEED0), (42u64, KAT_SEED42)] {
            let mut rng = SplitMix64::new(seed);
            for &want in &expected {
                assert_eq!(rng.next_u64(), want, "seed {seed}");
            }
        }
    }

    #[test]
    fn uniform01_and_exponential_known_answer() {
        let mut rng = SplitMix64::new(0);
        let u = sample::uniform01(&mut rng);
        assert_eq!(u, 0.8833108082136426);

        let mut rng = SplitMix64::new(0);
        let e = sample::exponential(&mut rng, 1.0);
        assert_eq!(e, 2.148241359348383);
    }

    #[test]
    fn uniform01_in_unit_interval() {
        let mut rng = SplitMix64::new(7);
        for _ in 0..10_000 {
            let u = sample::uniform01(&mut rng);
            assert!((0.0..1.0).contains(&u));
        }
    }

    #[test]
    fn next_u32_is_high_bits_of_next_u64() {
        let expected = (KAT_SEED0[0] >> 32) as u32;
        let mut rng = SplitMix64::new(0);
        assert_eq!(rng.next_u32(), expected);
    }

    #[test]
    fn same_seed_same_stream() {
        let mut a = SplitMix64::new(123);
        let mut b = SplitMix64::new(123);
        for _ in 0..1000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn reseed_restarts_stream() {
        let mut rng = SplitMix64::new(0);
        let first = rng.next_u64();
        let _ = rng.next_u64();
        rng.reseed(0);
        assert_eq!(rng.next_u64(), first);
        rng.set_seed(0);
        assert_eq!(rng.next_u64(), first);
    }

    #[test]
    fn exponential_mean_is_sane() {
        let mut rng = SplitMix64::new(99);
        let n = 200_000;
        let sum: f64 = (0..n).map(|_| sample::exponential(&mut rng, 5.0)).sum();
        let mean = sum / f64::from(n);
        assert!((mean - 5.0).abs() < 0.1, "mean was {mean}");
    }

    #[test]
    fn normal_consumes_two_draws_and_is_centered() {
        // Two normals consume four uniforms; verify the draw count is fixed.
        let mut a = SplitMix64::new(5);
        let _ = sample::normal(&mut a, 0.0, 1.0);
        let after_one = a.clone().next_u64();

        let mut b = SplitMix64::new(5);
        sample::uniform01(&mut b);
        sample::uniform01(&mut b);
        assert_eq!(after_one, b.next_u64());

        let mut rng = SplitMix64::new(1);
        let n = 200_000;
        let sum: f64 = (0..n).map(|_| sample::normal(&mut rng, 10.0, 2.0)).sum();
        let mean = sum / f64::from(n);
        assert!((mean - 10.0).abs() < 0.05, "mean was {mean}");
    }

    #[test]
    fn stdrng_reseed_is_deterministic() {
        let mut a = StdRng::seed_from_u64(0);
        a.reseed(77);
        let mut b = StdRng::seed_from_u64(77);
        assert_eq!(a.next_u64(), b.next_u64());
    }
}
