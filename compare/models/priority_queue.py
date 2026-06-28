"""Two-class priority queue. SimPy twin of `compare --model priority`.

Uses `simpy.PriorityResource` (lower priority value served first, matching
simu's PriorityResource). Each arrival is high (0) or low (1) with prob 0.5.
"""

import simpy

from _common import run
from _feed import SplitMix64


def run_seed(seed, args):
    n, lam, mu, servers = args.n, args.lam, args.mu, args.servers
    rng = SplitMix64(seed)
    env = simpy.Environment()
    server = simpy.PriorityResource(env, capacity=servers)
    recs = []  # (prio, wait)

    def customer(prio, arrival, service):
        with server.request(priority=prio) as req:
            yield req
            wait = env.now - arrival
            yield env.timeout(service)
        recs.append((prio, wait))

    def arrivals():
        for _ in range(n):
            yield env.timeout(rng.exponential(1.0 / lam))
            arrival = env.now
            service = rng.exponential(1.0 / mu)
            prio = 0 if rng.bernoulli(0.5) else 1
            env.process(customer(prio, arrival, service))

    env.process(arrivals())
    env.run()

    def mean_of(cls):
        xs = [w for (p, w) in recs if cls is None or p == cls]
        return sum(xs) / len(xs) if xs else 0.0

    return {
        "mean_wait_high": mean_of(0),
        "mean_wait_low": mean_of(1),
        "mean_wait_all": mean_of(None),
    }


def events_per_seed(rec, args):
    return 2 * args.n


if __name__ == "__main__":
    run("priority", run_seed, events_per_seed)
