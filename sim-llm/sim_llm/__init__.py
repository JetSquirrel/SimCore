# SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
#
# SPDX-License-Identifier: MIT OR Apache-2.0

"""sim-llm: LLM-provider workload simulation on the SimCore DES kernel.

Layered architecture:

- ``simcore`` (Rust kernel + PyO3 binding): deterministic discrete-event
  primitives — time, processes, priority resources, token containers, RNG.
- ``sim_llm`` (this package, pure Python): LLM-domain *policies* —
  rate-limit refill, budgets, latency models, workload generation, metrics.
"""

from .agent import Agent
from .metrics import AgentStats, Metrics, ProviderStats
from .provider import CompletionResult, Provider

__all__ = [
    "Agent",
    "AgentStats",
    "CompletionResult",
    "Metrics",
    "Provider",
    "ProviderStats",
]
