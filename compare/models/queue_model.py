# SPDX-FileCopyrightText: 2026 Christoph Kuhmuench <christoph.kuhmuench@gmail.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

"""Shared M/M/c queue model used by mm1.py (servers=1) and mmc.py.

Mirrors `run_queue` in examples/compare.rs: Exp(lambda) inter-arrivals, Exp(mu)
service, a `simpy.Resource(capacity=servers)`. Metrics are computed from the
same per-customer (wait, service, departure) records so both tools agree on the
definitions:
  * mean_wait    = mean time in queue before service
  * mean_sojourn = mean(wait + service)
  * mean_queue   = mean number in system L = sum(sojourn) / horizon
  * utilization  = busy server-time / (servers * horizon)
  * throughput   = customers / horizon
"""

import simpy

from _feed import SplitMix64


def run_queue_seed(seed, n, lam, mu, servers):
    rng = SplitMix64(seed)
    env = simpy.Environment()
    server = simpy.Resource(env, capacity=servers)
    recs = []  # (wait, service, departure)

    def customer(arrival, service):
        with server.request() as req:
            yield req
            wait = env.now - arrival
            yield env.timeout(service)
        recs.append((wait, service, env.now))

    def arrivals():
        for _ in range(n):
            yield env.timeout(rng.exponential(1.0 / lam))
            arrival = env.now
            service = rng.exponential(1.0 / mu)
            env.process(customer(arrival, service))

    env.process(arrivals())
    env.run()

    count = len(recs)
    if count == 0:
        return {
            "mean_wait": 0.0, "mean_sojourn": 0.0, "mean_queue": 0.0,
            "utilization": 0.0, "throughput": 0.0,
        }
    t_end = max(r[2] for r in recs)
    sum_wait = sum(r[0] for r in recs)
    sum_service = sum(r[1] for r in recs)
    sum_sojourn = sum(r[0] + r[1] for r in recs)
    return {
        "mean_wait": sum_wait / count,
        "mean_sojourn": sum_sojourn / count,
        "mean_queue": sum_sojourn / t_end if t_end > 0 else 0.0,
        "utilization": sum_service / (servers * t_end) if t_end > 0 else 0.0,
        "throughput": count / t_end if t_end > 0 else 0.0,
    }
