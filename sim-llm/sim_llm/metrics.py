# SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
#
# SPDX-License-Identifier: MIT OR Apache-2.0

"""Statistics collection and reporting for sim-llm.

Metrics are pure model-layer accounting: the simcore kernel knows nothing
about costs, budgets, or tokens-per-dollar. Providers and agents record
into their own stats objects; :class:`Metrics` aggregates them into a
printable summary.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from .agent import Agent
    from .provider import Provider


@dataclass
class ProviderStats:
    """Per-provider counters, updated once per completed call."""

    requests: int = 0
    tokens: int = 0
    cost: float = 0.0
    wait_total: float = 0.0  # queueing time (rate gates + slot), summed
    rate_wait_total: float = 0.0  # token-gate component (FIFO)
    slot_wait_total: float = 0.0  # slot-gate component (priority-ordered)
    wait_max: float = 0.0
    busy: float = 0.0  # seconds holding a concurrency slot (for utilization)

    def record(
        self,
        tokens: int,
        cost: float,
        wait: float,
        rate_wait: float,
        slot_wait: float,
        service: float,
    ) -> None:
        self.requests += 1
        self.tokens += tokens
        self.cost += cost
        self.wait_total += wait
        self.rate_wait_total += rate_wait
        self.slot_wait_total += slot_wait
        self.wait_max = max(self.wait_max, wait)
        self.busy += service

    @property
    def wait_avg(self) -> float:
        return self.wait_total / self.requests if self.requests else 0.0

    @property
    def rate_wait_avg(self) -> float:
        return self.rate_wait_total / self.requests if self.requests else 0.0

    @property
    def slot_wait_avg(self) -> float:
        return self.slot_wait_total / self.requests if self.requests else 0.0


@dataclass
class AgentStats:
    """Per-agent counters, updated once per completed call."""

    requests: int = 0
    tokens: int = 0
    cost: float = 0.0
    wait_total: float = 0.0
    wait_max: float = 0.0
    budget_skips: int = 0  # requests not issued because they would overrun

    def record(self, tokens: int, cost: float, wait: float) -> None:
        self.requests += 1
        self.tokens += tokens
        self.cost += cost
        self.wait_total += wait
        self.wait_max = max(self.wait_max, wait)

    @property
    def wait_avg(self) -> float:
        return self.wait_total / self.requests if self.requests else 0.0


class Metrics:
    """Aggregates provider/agent stats into a compact summary table."""

    def __init__(self, providers: list[Provider], agents: list[Agent]) -> None:
        self.providers = providers
        self.agents = agents

    def summary(self, now: float) -> str:
        lines: list[str] = [f"simulation time: {now:.2f}s", "", "providers:"]
        lines.append(
            f"  {'name':<10} {'req':>5} {'tokens':>8} {'cost':>8} "
            f"{'tok_wait':>9} {'slot_wait':>10} {'wait_max':>9} {'util':>6}"
        )
        for p in self.providers:
            s = p.stats
            util = s.busy / (p.concurrency * now) if now > 0 else 0.0
            lines.append(
                f"  {p.name:<10} {s.requests:>5} {s.tokens:>8} ${s.cost:>7.3f} "
                f"{s.rate_wait_avg:>8.3f}s {s.slot_wait_avg:>9.3f}s "
                f"{s.wait_max:>8.3f}s {util:>5.0%}"
            )
        lines.append("")
        lines.append("agents:")
        lines.append(
            f"  {'name':<14} {'prio':>5} {'provider':<10} {'req':>5} {'tokens':>8} "
            f"{'cost':>8} {'budget':>8} {'used':>6} {'wait_avg':>9} {'skips':>6}"
        )
        for a in self.agents:
            s = a.stats
            used = s.cost / a.budget if a.budget > 0 else 0.0
            lines.append(
                f"  {a.name:<14} {a.priority:>5} {a.provider.name:<10} {s.requests:>5} "
                f"{s.tokens:>8} ${s.cost:>7.3f} ${a.budget:>7.2f} {used:>5.0%} "
                f"{s.wait_avg:>8.3f}s {s.budget_skips:>6}"
            )
        return "\n".join(lines)
