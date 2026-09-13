# SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
#
# SPDX-License-Identifier: MIT OR Apache-2.0

# sim-llm

LLM-provider workload simulation on the [SimCore](../README.md)
discrete-event kernel. sim-llm is a **pure Python** package: it contains no
simulation engine of its own, only LLM-domain *policies* layered on top of
the `simcore` PyO3 binding (`simcore-python/`).

## Layered architecture

```
simcore kernel (Rust)      time, event queue, seeded RNG, PriorityResource, Container
        |
simcore binding (PyO3)     Python processes (async def / generators) driven by
        |                  the Rust executor — no asyncio
sim-llm (this package)     rate-limit refill policy, budgets, latency models,
                           workload generation, metrics  ← all model layer
```

What maps to what:

| Concept | Implementation | Layer |
|---|---|---|
| Concurrency cap | `simcore.PriorityResource` | kernel primitive |
| Call priority | `priority=` at the slot queue (lower = first) | kernel primitive |
| TPM / RPM limits | `simcore.Container` token bucket | kernel primitive |
| Bucket refill | per-second top-up process (`rate / 60` every 1 s) | **model policy** |
| Latency | `sim.timeout` of a sampled value | **model policy** |
| Price / budget | dollar accounting; agent stops before overrunning | **model policy** |
| Metrics | counters aggregated into a summary table | **model policy** |

## Quickstart

The only runtime dependency is the sibling `simcore` extension module
(not on PyPI). Build it once, then run:

```sh
cd simcore-python
python3 -m venv .venv && .venv/bin/pip install maturin
.venv/bin/maturin develop --release

cd ..
PYTHONPATH=sim-llm simcore-python/.venv/bin/python sim-llm/examples/llm_scheduling.py
# or install the package instead of using PYTHONPATH:
#   simcore-python/.venv/bin/pip install -e sim-llm
```

Determinism: same seed, byte-identical output.

```sh
PYTHONPATH=sim-llm simcore-python/.venv/bin/python sim-llm/examples/llm_scheduling.py > run1.txt
PYTHONPATH=sim-llm simcore-python/.venv/bin/python sim-llm/examples/llm_scheduling.py > run2.txt
diff run1.txt run2.txt   # no output
```

## API sketch

```python
import simcore
from sim_llm import Agent, Metrics, Provider

sim = simcore.Simulation(seed=42)
claude = Provider(sim, "claude", concurrency=3, tpm=200_000, rpm=600,
                  latency=(3.0, 6.0), price=15e-6)
agent = Agent(sim, "premium-0", claude, priority=10, budget=5.0,
              arrival_mean=4.0, tokens=(1500, 2500))
sim.run_until(180.0)
print(Metrics(providers=[claude], agents=[agent]).summary(sim.now()))
```

- `Provider(sim, name, *, concurrency, tpm=None, rpm=None, latency, price)`
  — `latency` is a fixed number of seconds, a `(lo, hi)` uniform range, or
  a callable `sim -> seconds`. `await provider.completion(tokens, priority=…)`
  gates on RPM, then TPM, then the priority-ordered concurrency slot (see
  docstring for the acquisition-order rationale), holds the slot for the
  sampled latency, and returns a `CompletionResult`.
- `Agent(sim, name, provider, *, priority=100, budget, arrival_mean, tokens,
  max_requests=None)` — Poisson arrivals (exponential via the kernel RNG),
  fixed or `(min, max)` request sizes, and a hard budget gate: the agent
  stops *before* a request that would exceed its budget.
- `Metrics(providers, agents).summary(now)` — per-provider table (requests,
  tokens, cost, token-gate vs slot-gate wait, utilization) and per-agent
  table (requests, cost vs budget, wait, budget skips).

## Limitations (honest, by design in v1)

- **The token bucket is FIFO, not priority-aware.** A low-priority agent
  blocked head-of-line on TPM delays every waiter behind it regardless of
  their priority; priority ordering applies only at the concurrency slot
  gate. Priority-aware rate limiting is model-layer policy for a later
  version (e.g. one bucket per traffic class).
- **No preemption.** A running low-priority call cannot be evicted by a
  premium arrival. The kernel has `PreemptiveResource` but the binding does
  not expose it yet — a follow-up.
- **Latency is a single timeout.** No token-by-token streaming, no
  TTFT/TBT distinction.
- **Single provider per agent.** Multi-provider routing/failover is a
  follow-up; build it as model policy, not kernel machinery.
