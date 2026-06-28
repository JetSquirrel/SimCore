# simu vs. SimPy comparison harness

Cross-checks the `simu` Rust DES engine against Python's
[SimPy](https://simpy.readthedocs.io/) on two axes:

1. **Correctness** — treating SimPy as a reference oracle, are the two engines'
   output-metric *distributions* statistically indistinguishable across many
   seeds? (Canonical queue models are additionally checked against closed-form
   queueing theory, so neither tool has to be trusted blindly.)
2. **Performance / efficiency** — wall-clock time, events/sec, and peak resident
   memory for each engine on the same workloads.

Both engines emit the **same JSON contract**, so the harness compares them
directly. Because Rust's `StdRng` (ChaCha12) and NumPy draw different number
streams from the same seed, **bit-identical traces are not expected** — the
comparison is at the level of distributions across replications, which is robust
to RNG differences.

## Layout

```
compare/
├── requirements.txt          # simpy, numpy, scipy
├── models/                   # SimPy models (one JSON-emitting script each)
│   ├── _common.py            # CLI parsing + JSON emission + seed loop
│   ├── queue_model.py        # shared M/M/c logic
│   ├── mm1.py  mmc.py  priority_queue.py  container.py  hospital.py
├── harness/
│   ├── contract.py           # subprocess runners + result schema + RSS/timing
│   ├── run_correctness.py    # distribution tests + Erlang-C ground truth
│   ├── run_performance.py    # wall-clock / events-per-sec / peak memory
│   └── report.py             # writes REPORT.md, exit non-zero on FAIL
└── run_comparison.sh         # build Rust, set up venv, run everything
```

The Rust side is `examples/compare.rs` (built with
`cargo build --release --example compare`). It shares no code with the harness —
it is a pure consumer of the public `simu` API and prints one JSON object.

## Running

```bash
# Full run (builds Rust, creates venv, writes compare/REPORT.md):
compare/run_comparison.sh

# Quick smoke run with fewer seeds/arrivals:
compare/run_comparison.sh --scale 0.1 --perf-scale 0.05

# Correctness only:
compare/run_comparison.sh --no-perf
```

Or drive pieces directly after `source compare/.venv/bin/activate`:

```bash
cargo build --release --example compare
PYTHONPATH=compare/harness:compare/models python compare/harness/run_correctness.py --scale 0.5
PYTHONPATH=compare/harness:compare/models python compare/harness/run_performance.py --scale 0.2
```

## Models and the JSON contract

Every run prints one line:

```json
{"tool":"simu","model":"mm1","seeds":1000,"n":1000,"lambda":0.9,"mu":1.0,
 "servers":1,"events":2000000,"wall_secs":0.12,"per_seed":[{...}, ...]}
```

| model       | primitive exercised        | per-seed metrics |
|-------------|----------------------------|------------------|
| `mm1`       | `Resource(1)`              | mean_wait, mean_sojourn, mean_queue, utilization, throughput |
| `mmc`       | `Resource(c)`              | (same) |
| `priority`  | `PriorityResource`         | mean_wait_high / _low / _all |
| `container` | `Container` (mixed sizes)  | mean_wait_all / _small / _large, served |
| `hospital`  | port of `examples/hospital.rs` | the 8 hospital metrics |

`events` uses an identical, model-specific definition on both sides (e.g. `2·N`
for the queue models: one arrival + one departure transition per customer) so
events/sec is comparable across tools.

### Metric definitions (computed identically on both sides)
- `mean_wait` — mean time in queue before service.
- `mean_queue` — mean number in system, `L = Σ sojourn / horizon` (area under
  `N(t)` equals the sum of sojourn times).
- `utilization` — busy server-time `/ (servers · horizon)`.

## Correctness methodology

For each metric we take the per-seed arrays from both tools and compare
distributions:

- **Welch's t-test** on the means (`scipy.stats.ttest_ind`, `equal_var=False`).
- **Two-sample KS test** on the full distribution.
- **Relative mean difference** (effect size).

A metric **FAILS only when the difference is both statistically significant
(t-test p < α=0.01) and material (relative mean difference > tol=5%)**. This
"significant **and** material" gate avoids the large-sample trap where a
negligible difference becomes significant purely because there are many seeds.

For `mm1`/`mmc` the report also prints the closed-form Erlang-C mean wait; both
engines should land near it.

## Resolved finding: `Container` strict-FIFO

Earlier runs of this harness surfaced a real semantic difference (not a harness
flaw): simu's `Container` granted a *freshly-arriving* `get`/`put` immediately
whenever the current level/space covered it, bypassing already-queued larger
waiters, whereas SimPy's `Container` is strict head-of-line FIFO (while a large
`get` at the front is unsatisfiable, every later `get` waits behind it, even ones
that would fit). Small draws therefore skipped ahead of blocked large draws
(small waits ≪ large waits), while SimPy makes both wait equally:

| tool  | mean_wait_small | mean_wait_large |
|-------|-----------------|-----------------|
| simu (pre-fix) | ~22    | ~149            |
| simpy          | ~59    | ~59             |

**Fixed** in `src/resource/container.rs`: the immediate-completion fast paths in
`ContainerGetRequest::poll` / `ContainerPutRequest::poll` are now gated on
`has_live_get_waiter` / `has_live_put_waiter`, so a fresh request never takes
level/space ahead of a live same-kind waiter. The `container` model now **passes**
— `mean_wait_small` ≈ `mean_wait_large` ≈ SimPy — and the Container-driven
hospital metrics (`blood_bank_waits`, `mean_blood_wait`, `critical_treated`) come
into agreement too. See the `container` section of `REPORT.md`.

## Open finding: hospital `early_discharged`

A separate, smaller discrepancy remains in the `hospital` model: simu reports more
`early_discharged` patients than SimPy (~4.1 vs ~3.5, ~15% relative; significant
on the means though not on the KS distribution test). This is **unrelated to the
Container FIFO fix** — every Container-driven hospital metric now passes. It
points to a difference in the bed preemption / early-discharge path between
`examples/hospital.rs` and the SimPy port (`models/hospital.py`), and is still
under investigation.
