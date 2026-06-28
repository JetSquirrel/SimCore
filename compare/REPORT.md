# simu vs. SimPy comparison report

_Generated 2026-06-28T14:39:04_

## Correctness: simu vs. SimPy (distribution equivalence)

FAIL = statistically significant (t-test p < 0.01) **and** material (relative mean difference > 5%).

### mm1  (✅ PASS) — 200 seeds × 1000 arrivals, λ=0.85, μ=1.0, c=1

| metric | simu mean | simpy mean | rel.diff | t-test p | KS p | result |
|---|---|---|---|---|---|---|
| mean_queue | 5.448 ± 0.16 | 5.47 ± 0.13 | 0.39% | 0.916 | 0.545 | pass |
| mean_sojourn | 6.432 ± 0.17 | 6.468 ± 0.15 | 0.56% | 0.873 | 0.545 | pass |
| mean_wait | 5.431 ± 0.17 | 5.466 ± 0.14 | 0.64% | 0.877 | 0.466 | pass |
| throughput | 0.8431 ± 0.0018 | 0.8422 ± 0.0018 | 0.10% | 0.740 | 0.713 | pass |
| utilization | 0.8439 ± 0.0026 | 0.8443 ± 0.0025 | 0.05% | 0.904 | 0.793 | pass |

_Analytical mean wait (Erlang C) = **5.667**; simu = 5.431, simpy = 5.466._

### mmc  (✅ PASS) — 200 seeds × 1000 arrivals, λ=1.6, μ=1.0, c=2

| metric | simu mean | simpy mean | rel.diff | t-test p | KS p | result |
|---|---|---|---|---|---|---|
| mean_queue | 4.354 ± 0.087 | 4.406 ± 0.077 | 1.18% | 0.653 | 0.713 | pass |
| mean_sojourn | 2.733 ± 0.051 | 2.77 ± 0.045 | 1.36% | 0.580 | 0.713 | pass |
| mean_wait | 1.732 ± 0.05 | 1.768 ± 0.044 | 2.04% | 0.587 | 0.793 | pass |
| throughput | 1.588 ± 0.0034 | 1.586 ± 0.0035 | 0.14% | 0.642 | 0.545 | pass |
| utilization | 0.795 ± 0.0025 | 0.7951 ± 0.0024 | 0.01% | 0.982 | 0.866 | pass |

_Analytical mean wait (Erlang C) = **1.778**; simu = 1.732, simpy = 1.768._

### priority  (✅ PASS) — 200 seeds × 1000 arrivals, λ=0.85, μ=1.0, c=1

| metric | simu mean | simpy mean | rel.diff | t-test p | KS p | result |
|---|---|---|---|---|---|---|
| mean_wait_all | 5.418 ± 0.16 | 5.424 ± 0.17 | 0.11% | 0.979 | 0.793 | pass |
| mean_wait_high | 1.461 ± 0.015 | 1.495 ± 0.014 | 2.29% | 0.099 | 0.142 | pass |
| mean_wait_low | 9.334 ± 0.3 | 9.34 ± 0.32 | 0.06% | 0.990 | 0.965 | pass |

### container  (✅ PASS) — 200 seeds × 1000 arrivals, λ=0.9, μ=1.0, c=1

| metric | simu mean | simpy mean | rel.diff | t-test p | KS p | result |
|---|---|---|---|---|---|---|
| mean_wait_all | 28.8 ± 1.5 | 29.24 ± 1.6 | 1.52% | 0.842 | 0.988 | pass |
| mean_wait_large | 29.47 ± 1.5 | 29.86 ± 1.6 | 1.32% | 0.859 | 0.793 | pass |
| mean_wait_small | 28.51 ± 1.6 | 28.98 ± 1.6 | 1.60% | 0.836 | 0.988 | pass |
| served | 948.3 ± 2.3 | 946.8 ± 2.4 | 0.15% | 0.663 | 0.466 | pass |

### hospital  (❌ FAIL) — 200 seeds × 1000 arrivals, λ=0.9, μ=1.0, c=1

| metric | simu mean | simpy mean | rel.diff | t-test p | KS p | result |
|---|---|---|---|---|---|---|
| blood_bank_waits | 17.93 ± 0.67 | 18.12 ± 0.72 | 1.02% | 0.850 | 0.965 | pass |
| critical_treated | 14.35 ± 0.15 | 14.46 ± 0.12 | 0.73% | 0.577 | 0.924 | pass |
| early_discharged | 4.09 ± 0.14 | 3.485 ± 0.13 | 14.79% | 0.002 | 0.221 | **FAIL** |
| ed_cleared_at | 527 ± 1.9 | 526.9 ± 2.1 | 0.01% | 0.979 | 0.965 | pass |
| mean_bed_wait | 4.187 ± 0.24 | 4.049 ± 0.21 | 3.28% | 0.666 | 0.793 | pass |
| mean_blood_wait | 25.41 ± 1.6 | 25.37 ± 1.5 | 0.18% | 0.984 | 0.924 | pass |
| mean_nurse_wait | 3.604 ± 0.12 | 3.785 ± 0.14 | 4.78% | 0.324 | 0.713 | pass |
| standard_treated | 35.02 ± 0.51 | 34.84 ± 0.45 | 0.50% | 0.797 | 0.793 | pass |

## Performance: simu vs. SimPy

| model | arrivals | tool | wall (s) | events/sec | peak RSS (MB) |
|---|---|---|---|---|---|
| mm1 | 100,000 | simu | 0.082 | 2,426,245 | 4.1 |
| mm1 | 100,000 | simpy | 0.842 | 237,526 | 54.8 |
| mmc | 100,000 | simu | 0.080 | 2,484,602 | 4.1 |
| mmc | 100,000 | simpy | 0.857 | 233,342 | 54.8 |
| priority | 100,000 | simu | 0.087 | 2,291,842 | 4.3 |
| priority | 100,000 | simpy | 0.987 | 202,720 | 48.0 |

| model | simu speedup × | simu memory advantage × |
|---|---|---|
| mm1 | 10.2× | 13.2× |
| mmc | 10.6× | 13.2× |
| priority | 11.3× | 11.2× |

