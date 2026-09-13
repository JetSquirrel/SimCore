# SPDX-FileCopyrightText: Copyright (c) Siemens 2026 contributed by Christoph Kuhmuench christoph.kuhmuench@gmail.com
#
# SPDX-License-Identifier: MIT OR Apache-2.0

"""LLM provider endpoint model.

Layer map (kernel primitive vs model-layer policy):

- Concurrency cap  -> KERNEL: ``simcore.PriorityResource``; ``priority=``
  orders the slot queue (lower number = served first).
- TPM / RPM limits -> KERNEL primitive ``simcore.Container`` (token bucket)
  driven by a MODEL-LAYER refill process: a plain sim process that tops the
  bucket up continuously (rate / 60 every simulated second). The refill
  *policy* — amount, interval, burst capacity — lives here, not in the
  kernel.
- Latency          -> MODEL: a simple ``sim.timeout`` sample. No streaming,
  no TTFT/TBT distinction (v1 boundary).
- Price            -> MODEL: dollars per token, pure accounting.
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass

import simcore

from .metrics import ProviderStats

#: Latency spec: fixed seconds, a (lo, hi) uniform range, or a callable
#: sampled per call (receives the Simulation, returns seconds).
LatencySpec = float | tuple[float, float] | Callable[["simcore.Simulation"], float]

SECONDS_PER_MINUTE = 60.0


@dataclass(frozen=True)
class CompletionResult:
    """What one ``completion()`` call resolved to."""

    provider: str
    tokens: int
    cost: float
    wait: float  # total queueing delay (rate gates + concurrency slot)
    rate_wait: float  # delay at the RPM/TPM token gates (FIFO, no priority)
    slot_wait: float  # delay at the concurrency gate (priority-ordered)
    latency: float  # service time holding the slot


class Provider:
    """An LLM endpoint with a concurrency cap, optional rate limits, and a price.

    Args:
        sim: the simulation environment.
        name: display name.
        concurrency: max parallel in-flight calls (kernel PriorityResource).
        tpm: tokens-per-minute limit, or None for unlimited. The bucket
            capacity equals ``tpm`` (one minute of burst), refilled
            continuously at ``tpm / 60`` per simulated second.
        rpm: requests-per-minute limit, or None; same refill semantics.
        latency: fixed seconds, ``(lo, hi)`` uniform range, or callable.
        price: dollars per token.
    """

    def __init__(
        self,
        sim: "simcore.Simulation",
        name: str,
        *,
        concurrency: int,
        tpm: float | None = None,
        rpm: float | None = None,
        latency: LatencySpec,
        price: float,
    ) -> None:
        if concurrency < 1:
            raise ValueError("concurrency must be at least 1")
        if price < 0:
            raise ValueError("price must be non-negative")
        self._sim = sim
        self.name = name
        self.concurrency = concurrency
        self.tpm = tpm
        self.rpm = rpm
        self.latency = latency
        self.price = price
        self.stats = ProviderStats()

        self._slots = simcore.PriorityResource(capacity=concurrency)
        self._tpm_bucket = (
            simcore.Container(capacity=float(tpm), init=float(tpm)) if tpm else None
        )
        self._rpm_bucket = (
            simcore.Container(capacity=float(rpm), init=float(rpm)) if rpm else None
        )
        if self._tpm_bucket is not None:
            sim.spawn(self._refill(self._tpm_bucket, tpm / SECONDS_PER_MINUTE))
        if self._rpm_bucket is not None:
            sim.spawn(self._refill(self._rpm_bucket, rpm / SECONDS_PER_MINUTE))

    async def _refill(self, bucket: "simcore.Container", amount: float) -> None:
        """Continuous top-up: `amount` every simulated second.

        Chosen over a per-minute window refill because it avoids the
        thundering-herd at each minute boundary and keeps queueing behavior
        smooth and deterministic. The kernel Container blocks the put when
        the bucket is full, which is exactly leaky-bucket semantics.
        """
        while True:
            await self._sim.timeout(1.0)
            await bucket.put(amount)

    def _sample_latency(self) -> float:
        spec = self.latency
        if callable(spec):
            return float(spec(self._sim))
        if isinstance(spec, tuple):
            lo, hi = spec
            return self._sim.rng_uniform(lo, hi)
        return float(spec)

    async def completion(self, tokens: int, priority: int = 100) -> CompletionResult:
        """Run one completion call.

        Acquisition order: rate gates first (RPM, then TPM), then the
        concurrency slot. Rationale: a concurrency slot held while blocked
        on tokens is capacity other callers cannot use — gating on tokens
        first keeps scarce slots for calls that can actually start. The
        tradeoff is that tokens are *deducted* before the slot wait begins
        (rate accounting happens at admission, not at call start), and the
        kernel Container is strict FIFO — a large head-of-line `get` delays
        smaller ones regardless of priority. Priority ordering applies at
        the slot queue. (See README limitations.)
        """
        if tokens <= 0:
            raise ValueError(f"tokens must be positive (got {tokens})")
        t0 = self._sim.now()
        if self._rpm_bucket is not None:
            await self._rpm_bucket.get(1.0)
        if self._tpm_bucket is not None:
            await self._tpm_bucket.get(float(tokens))
        rate_wait = self._sim.now() - t0
        async with self._slots.acquire(priority=priority):
            slot_wait = self._sim.now() - t0 - rate_wait
            latency = self._sample_latency()
            await self._sim.timeout(latency)
        cost = tokens * self.price
        wait = rate_wait + slot_wait
        self.stats.record(
            tokens=tokens, cost=cost, wait=wait,
            rate_wait=rate_wait, slot_wait=slot_wait, service=latency,
        )
        return CompletionResult(
            provider=self.name, tokens=tokens, cost=cost, wait=wait,
            rate_wait=rate_wait, slot_wait=slot_wait, latency=latency,
        )
