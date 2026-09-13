# SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
#
# SPDX-License-Identifier: MIT OR Apache-2.0

"""LLM scheduling scenario: mixed-priority agents on two providers.

Two providers with different bottleneck shapes:
- "claude": expensive, generous TPM, tight concurrency (3 slots) — the
  priority-ordered SLOT gate is the bottleneck (premium vs standard
  visible in per-agent wait times).
- "openai": cheap, tight TPM, wide concurrency (8 slots) — the FIFO TOKEN
  bucket is the bottleneck.

Three agent classes — premium (prio 10), standard (prio 50), batch
(prio 100) — with different budgets and workload intensities, run over a
fixed 180-second window (run_until). Deterministic: same seed, identical
output. Run twice and diff to verify.

Usage:
    PYTHONPATH=sim-llm python sim-llm/examples/llm_scheduling.py
(or `pip install -e sim-llm` into the venv and run it directly)
"""

from sim_llm import Agent, Metrics, Provider

import simcore

WINDOW_S = 180.0


def main() -> None:
    sim = simcore.Simulation(seed=42)

    claude = Provider(
        sim,
        "claude",
        concurrency=3,
        tpm=200_000,  # generous: the slot gate, not tokens, binds here
        rpm=600,
        latency=(3.0, 6.0),  # seconds, uniform
        price=15e-6,  # $15 / 1M tokens
    )
    openai = Provider(
        sim,
        "openai",
        concurrency=8,
        tpm=24_000,
        rpm=300,
        latency=(0.4, 1.0),
        price=2e-6,  # $2 / 1M tokens
    )

    agents = [
        # Interactive premium traffic on claude: low volume, big requests.
        Agent(sim, "premium-0", claude, priority=10, budget=5.0,
              arrival_mean=4.0, tokens=(1500, 2500)),
        Agent(sim, "premium-1", claude, priority=10, budget=5.0,
              arrival_mean=5.0, tokens=(1500, 2500)),
        # Standard traffic on claude: higher volume, competes for slots —
        # six agents on three slots keep the priority queue non-empty.
        Agent(sim, "standard-0", claude, priority=50, budget=3.0,
              arrival_mean=2.0, tokens=(1000, 3000)),
        Agent(sim, "standard-1", claude, priority=50, budget=3.0,
              arrival_mean=2.5, tokens=(1000, 3000)),
        Agent(sim, "standard-2", claude, priority=50, budget=3.0,
              arrival_mean=2.2, tokens=(1000, 3000)),
        Agent(sim, "standard-3", claude, priority=50, budget=3.0,
              arrival_mean=2.8, tokens=(1000, 3000)),
        # Batch jobs on openai: high volume, small requests, tight budget —
        # the budget gate cuts them off before the window ends.
        Agent(sim, "batch-0", openai, priority=100, budget=0.08,
              arrival_mean=2.0, tokens=(500, 1500)),
        Agent(sim, "batch-1", openai, priority=100, budget=0.08,
              arrival_mean=2.5, tokens=(500, 1500)),
    ]

    metrics = Metrics(providers=[claude, openai], agents=agents)
    sim.run_until(WINDOW_S)
    print(metrics.summary(sim.now()))


if __name__ == "__main__":
    main()
