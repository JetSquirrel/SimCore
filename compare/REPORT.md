# simu vs. SimPy comparison report

_Generated 2026-06-28T18:21:06_

## Correctness: simu vs. SimPy

Both tools draw from the same portable feed (SplitMix64 + shared transforms), seeded identically per replication.

- **Exact mode** (mm1, mmc, priority, container): every metric must agree **seed-by-seed** within 1e-9 relative; FAIL otherwise.
- **Distributional mode** (hospital): FAIL = statistically significant (t-test p < 0.01) **and** material (relative mean difference > 5%); the `early_discharged` metric is a documented eviction-handoff ordering exception (see README).

### mm1  (✅ PASS, exact) — 200 seeds × 1000 arrivals, λ=0.85, μ=1.0, c=1

| metric | simu mean | simpy mean | rel.diff | max/seed Δ | t-test p | KS p | result |
|---|---|---|---|---|---|---|---|
| mean_queue | 5.299 ± 0.15 | 5.299 ± 0.15 | 0.00% | 2.9e-15 | 1.000 | 1.000 | pass |
| mean_sojourn | 6.232 ± 0.16 | 6.232 ± 0.16 | 0.00% | 3.0e-15 | 1.000 | 1.000 | pass |
| mean_wait | 5.237 ± 0.16 | 5.237 ± 0.16 | 0.00% | 2.9e-15 | 1.000 | 1.000 | pass |
| throughput | 0.8457 ± 0.0018 | 0.8457 ± 0.0018 | 0.00% | 0.0e+00 | 1.000 | 1.000 | pass |
| utilization | 0.8422 ± 0.0028 | 0.8422 ± 0.0028 | 0.00% | 2.2e-15 | 1.000 | 1.000 | pass |

_Analytical mean wait (Erlang C) = **5.667**; simu = 5.237, simpy = 5.237._

### mmc  (✅ PASS, exact) — 200 seeds × 1000 arrivals, λ=1.6, μ=1.0, c=2

| metric | simu mean | simpy mean | rel.diff | max/seed Δ | t-test p | KS p | result |
|---|---|---|---|---|---|---|---|
| mean_queue | 4.284 ± 0.079 | 4.284 ± 0.079 | 0.00% | 2.5e-15 | 1.000 | 1.000 | pass |
| mean_sojourn | 2.681 ± 0.046 | 2.681 ± 0.046 | 0.00% | 2.5e-15 | 1.000 | 1.000 | pass |
| mean_wait | 1.685 ± 0.044 | 1.685 ± 0.044 | 0.00% | 2.3e-15 | 1.000 | 1.000 | pass |
| throughput | 1.593 ± 0.0034 | 1.593 ± 0.0034 | 0.00% | 0.0e+00 | 1.000 | 1.000 | pass |
| utilization | 0.793 ± 0.0026 | 0.793 ± 0.0026 | 0.00% | 2.2e-15 | 1.000 | 1.000 | pass |

_Analytical mean wait (Erlang C) = **1.778**; simu = 1.685, simpy = 1.685._

### priority  (✅ PASS, exact) — 200 seeds × 1000 arrivals, λ=0.85, μ=1.0, c=1

| metric | simu mean | simpy mean | rel.diff | max/seed Δ | t-test p | KS p | result |
|---|---|---|---|---|---|---|---|
| mean_wait_all | 5.373 ± 0.16 | 5.373 ± 0.16 | 0.00% | 2.6e-15 | 1.000 | 1.000 | pass |
| mean_wait_high | 1.454 ± 0.015 | 1.454 ± 0.015 | 0.00% | 5.4e-16 | 1.000 | 1.000 | pass |
| mean_wait_low | 9.33 ± 0.32 | 9.33 ± 0.32 | 0.00% | 1.7e-15 | 1.000 | 1.000 | pass |

### container  (✅ PASS, exact) — 200 seeds × 1000 arrivals, λ=0.9, μ=1.0, c=1

| metric | simu mean | simpy mean | rel.diff | max/seed Δ | t-test p | KS p | result |
|---|---|---|---|---|---|---|---|
| mean_wait_all | 31.2 ± 1.6 | 31.2 ± 1.6 | 0.00% | 2.2e-15 | 1.000 | 1.000 | pass |
| mean_wait_large | 31.84 ± 1.6 | 31.84 ± 1.6 | 0.00% | 1.1e-15 | 1.000 | 1.000 | pass |
| mean_wait_small | 30.93 ± 1.6 | 30.93 ± 1.6 | 0.00% | 1.8e-15 | 1.000 | 1.000 | pass |
| served | 942.4 ± 2.4 | 942.4 ± 2.4 | 0.00% | 0.0e+00 | 1.000 | 1.000 | pass |

### hospital  (❌ FAIL, distributional) — 200 seeds × 1000 arrivals, λ=0.9, μ=1.0, c=1

| metric | simu mean | simpy mean | rel.diff | max/seed Δ | t-test p | KS p | result |
|---|---|---|---|---|---|---|---|
| blood_bank_waits | 17.52 ± 0.63 | 17.52 ± 0.63 | 0.00% | 0.0e+00 | 1.000 | 1.000 | pass |
| critical_treated | 14.62 ± 0.12 | 14.62 ± 0.12 | 0.00% | 0.0e+00 | 1.000 | 1.000 | pass |
| early_discharged | 3.965 ± 0.14 | 3.43 ± 0.14 | 13.49% | 1.0e+00 | 0.006 | 0.205 | **FAIL** |
| ed_cleared_at | 526.7 ± 2.1 | 527.8 ± 2.1 | 0.22% | 1.2e-01 | 0.695 | 1.000 | pass |
| mean_bed_wait | 4.044 ± 0.25 | 4.217 ± 0.25 | 4.11% | 7.0e-01 | 0.623 | 0.906 | pass |
| mean_blood_wait | 24.84 ± 1.5 | 24.84 ± 1.5 | 0.00% | 2.9e-16 | 1.000 | 1.000 | pass |
| mean_nurse_wait | 3.814 ± 0.14 | 3.814 ± 0.14 | 0.00% | 3.4e-16 | 1.000 | 1.000 | pass |
| standard_treated | 34.35 ± 0.49 | 34.35 ± 0.49 | 0.00% | 0.0e+00 | 1.000 | 1.000 | pass |

_Note: differences here are **expected**, not a bug. Both engines draw the identical random stream, so every metric that depends on the random values matches exactly. The diverging metrics (`early_discharged`, and its cascade into `mean_bed_wait` / `ed_cleared_at`) depend instead on the **order in which events scheduled at the same simulated timestamp fire** — which is implementation-specific and differs between simu (poll/waker) and SimPy (generator/callback) during the bed-eviction handoff. No random number is consumed at that decision point, so a shared feed cannot remove the difference. See `compare/README.md`._

## Performance: simu vs. SimPy

Queue models run one large replication (`seeds=1`, large `n`); the hospital model ignores `n` and runs a fixed-duration scenario, so its workload is scaled by replication count (`seeds`). The `events` column is the common, model-specific event tally and is identical across the two tools by construction.

| model | workload | events | tool | wall (s) | events/sec | peak RSS (MB) |
|---|---|---|---|---|---|---|
| mm1 | 100,000 arrivals | 200,000 | simu | 0.044 | 4,583,783 | 4.2 |
| mm1 | 100,000 arrivals | 200,000 | simpy | 0.457 | 437,165 | 41.1 |
| mmc | 100,000 arrivals | 200,000 | simu | 0.040 | 4,973,573 | 4.2 |
| mmc | 100,000 arrivals | 200,000 | simpy | 0.471 | 424,744 | 41.1 |
| priority | 100,000 arrivals | 200,000 | simu | 0.043 | 4,598,723 | 4.4 |
| priority | 100,000 arrivals | 200,000 | simpy | 0.563 | 355,476 | 34.7 |
| hospital | 2,000 seeds | 196,318 | simu | 0.098 | 1,994,122 | 5.3 |
| hospital | 2,000 seeds | 196,318 | simpy | 1.373 | 143,035 | 25.5 |

| model | simu speedup × | simu memory advantage × |
|---|---|---|
| mm1 | 10.5× | 9.7× |
| mmc | 11.7× | 9.7× |
| priority | 12.9× | 7.9× |
| hospital | 13.9× | 4.8× |

_Note: all seeds run **sequentially** in a single process for both tools — these figures measure single-thread engine efficiency, not parallelism. (simu can parallelize independent seeds across cores via `monte_carlo::run`, which CPython's GIL effectively denies SimPy, but that is intentionally not exercised here.) For the **hospital** row the memory advantage narrows as the seed count grows: simu accumulates one small result record per seed (linear), while SimPy's ~24 MB interpreter baseline dominates and masks its own per-seed growth — so the *ratio* shrinks even though simu's engine memory stays small. The per-event speedup is unaffected and stays in the same band as the other models._

## Monte Carlo: parallel seeds (simu) vs sequential (SimPy)

simu fans independent replications across threads via `monte_carlo::run` (rayon; 10 logical cores on this machine); SimPy runs them sequentially because CPython's GIL prevents thread parallelism. Wall-clock is the total time to complete **all** replications. Output is byte-identical to the sequential run (same seeds, results returned in seed order).

| model | replications | simu seq (s) | simu ‖ (s) | SimPy seq (s) | simu parallel speedup × | simu ‖ vs SimPy × |
|---|---|---|---|---|---|---|
| hospital | 2,000 seeds | 0.109 | 0.032 | 1.378 | 3.4× | 43.1× |
| mm1 | 300 seeds × 4,000 | 0.604 | 0.213 | 5.600 | 2.8× | 26.3× |

_The **simu ‖ vs SimPy** column is the headline cross-tool advantage when replications run in parallel: roughly the single-thread per-event speedup times the parallel scaling (which saturates near the ~10× logical-core count and is trimmed by per-run setup and memory bandwidth). This is the axis SimPy cannot follow — its replications are GIL-serialised._

