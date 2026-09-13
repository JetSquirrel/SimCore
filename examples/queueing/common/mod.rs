// SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Shared helpers for the queueing-theory validation examples.
//!
//! Every example in this directory simulates a classic queueing model and
//! checks the result against the closed-form analytical value from
//! Harchol-Balter, *Performance Modeling and Design of Computer Systems:
//! Queueing Theory in Action*. Each example runs several independent
//! replications (fresh `SimEnv` per seed) and reports the mean with a 95%
//! confidence interval next to the theory value.

use simcore::rng::sample;
use simcore::EnvHandle;

/// Standard normal / Student-t two-sided 97.5% critical values by degrees
/// of freedom (df 1..=30, then 2.0/1.96 for large df).
fn t_crit(df: usize) -> f64 {
    const T: [f64; 30] = [
        12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228, 2.201, 2.179, 2.160,
        2.145, 2.131, 2.120, 2.110, 2.101, 2.093, 2.086, 2.080, 2.074, 2.069, 2.064, 2.060, 2.056,
        2.052, 2.048, 2.045, 2.042,
    ];
    match df {
        0 => f64::NAN,
        1..=30 => T[df - 1],
        31..=60 => 2.0,
        _ => 1.96,
    }
}

/// Independent replication results for one metric.
#[derive(Default)]
pub struct Replications {
    values: Vec<f64>,
}

impl Replications {
    pub fn new() -> Self {
        Replications { values: Vec::new() }
    }

    pub fn push(&mut self, v: f64) {
        self.values.push(v);
    }

    pub fn mean(&self) -> f64 {
        self.values.iter().sum::<f64>() / self.values.len() as f64
    }

    /// Half-width of the 95% confidence interval of the mean.
    pub fn ci95_half(&self) -> f64 {
        let n = self.values.len();
        if n < 2 {
            return f64::NAN;
        }
        let mean = self.mean();
        let var = self.values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1) as f64;
        t_crit(n - 1) * (var / n as f64).sqrt()
    }
}

/// Time-averaged level (e.g. number in system, server busy) with warm-up
/// exclusion: level changes before `start` are tracked (so the level is
/// correct at `start`) but contribute no area before `start`.
pub struct TimeAvg {
    start: f64,
    last: f64,
    area: f64,
    level: f64,
}

impl TimeAvg {
    pub fn new(start: f64) -> Self {
        TimeAvg { start, last: start, area: 0.0, level: 0.0 }
    }

    /// Change the level by `delta` at time `now`.
    pub fn add(&mut self, now: f64, delta: f64) {
        if now <= self.start {
            self.level += delta;
            return;
        }
        self.area += self.level * (now - self.last);
        self.last = now;
        self.level += delta;
    }

    /// Time-averaged level over `[start, end]`.
    pub fn mean(&self, end: f64) -> f64 {
        (self.area + self.level * (end - self.last)) / (end - self.start)
    }
}

/// Print one metric row and return whether theory lies inside the CI.
pub fn check(metric: &str, rep: &Replications, theory: f64) -> bool {
    let mean = rep.mean();
    let half = rep.ci95_half();
    let inside = (theory - mean).abs() <= half;
    let err = if theory != 0.0 {
        (mean - theory).abs() / theory * 100.0
    } else {
        mean.abs()
    };
    println!(
        "{:<10} {:>10.4} ±{:>8.4} {:>12.4} {:>8.2}%   {}",
        metric,
        mean,
        half,
        theory,
        err,
        if inside { "PASS" } else { "FAIL" }
    );
    inside
}

/// Sample an exponential with the given mean from the environment's seeded
/// RNG stream (re-exported so examples have one import site).
pub fn expovariate(env: &EnvHandle, mean: f64) -> f64 {
    sample::exponential(&mut env.rng(), mean)
}
