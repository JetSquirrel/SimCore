# SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
#
# SPDX-License-Identifier: MIT OR Apache-2.0

"""Workload driver: an agent issuing completion calls against a provider.

Everything here is MODEL-LAYER policy: the inter-arrival process, the
request size distribution, and budget accounting. The kernel only sees
timeouts and provider request objects.

Budget semantics: the agent estimates the cost of its *next* request
(tokens sampled, priced at the provider's per-token rate) and stops
*BEFORE* issuing a request that would overrun its budget. The last
request never exceeds the budget; the agent simply goes idle.
"""

from __future__ import annotations

import simcore

from .metrics import AgentStats
from .provider import Provider


class Agent:
    """A workload driver with a priority and a dollar budget.

    Args:
        sim: the simulation environment.
        name: display name.
        provider: the endpoint to call (single provider in v1; routing /
            failover across providers is a follow-up).
        priority: queueing priority at the provider's concurrency gate
            (lower number = served first).
        budget: total dollars the agent may spend.
        arrival_mean: mean seconds between requests (exponential, drawn
            from the kernel RNG stream via ``sim.rng_exponential``).
        tokens: tokens per request — a fixed int or a ``(min, max)``
            inclusive range sampled with ``sim.rng_range_int``.
        max_requests: optional hard cap on issued requests.
    """

    def __init__(
        self,
        sim: "simcore.Simulation",
        name: str,
        provider: Provider,
        *,
        priority: int = 100,
        budget: float,
        arrival_mean: float,
        tokens: int | tuple[int, int],
        max_requests: int | None = None,
    ) -> None:
        if budget < 0:
            raise ValueError("budget must be non-negative")
        if arrival_mean <= 0:
            raise ValueError("arrival_mean must be positive")
        self._sim = sim
        self.name = name
        self.provider = provider
        self.priority = priority
        self.budget = budget
        self.arrival_mean = arrival_mean
        self.tokens = tokens
        self.max_requests = max_requests
        self.stats = AgentStats()
        sim.spawn(self._run())

    def _sample_tokens(self) -> int:
        if isinstance(self.tokens, tuple):
            lo, hi = self.tokens
            return self._sim.rng_range_int(lo, hi)
        return self.tokens

    async def _run(self) -> None:
        while self.max_requests is None or self.stats.requests < self.max_requests:
            await self._sim.timeout(self._sim.rng_exponential(self.arrival_mean))
            n_tokens = self._sample_tokens()
            est_cost = n_tokens * self.provider.price
            if self.stats.cost + est_cost > self.budget:
                self.stats.budget_skips += 1
                break  # stop BEFORE the overrunning request
            result = await self.provider.completion(n_tokens, priority=self.priority)
            self.stats.record(tokens=result.tokens, cost=result.cost, wait=result.wait)
