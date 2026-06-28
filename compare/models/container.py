"""Mixed-size FIFO container stress test. SimPy twin of `compare --model container`.

A producer drips fixed-size puts into an initially-empty container while
consumers draw a LARGE or SMALL amount. This isolates a head-of-line-blocking
question: when a large draw at the front of the queue cannot yet be satisfied,
must a later small draw that *currently fits* wait behind it?

`simpy.Container` enforces strict FIFO (the small draw waits). Comparing the
two tools here reveals whether simu agrees. Self-contained constants; ignores
--lambda/--mu/--servers.
"""

import simpy

from _common import run
from _feed import SplitMix64

CAP = 1000.0
ARRIVAL_SCALE = 1.0
SMALL = 2.0
LARGE = 20.0
P_LARGE = 0.3
PUT_AMT = 7.0
PUT_INTERVAL = 1.0


def run_seed(seed, args):
    n = args.n
    rng = SplitMix64(seed)
    env = simpy.Environment()
    cont = simpy.Container(env, capacity=CAP, init=0.0)
    recs = []  # (large, wait)

    def consumer(amount, large, arrival):
        yield cont.get(amount)
        recs.append((large, env.now - arrival))

    def arrivals():
        for _ in range(n):
            yield env.timeout(rng.exponential(ARRIVAL_SCALE))
            arrival = env.now
            large = rng.bernoulli(P_LARGE)
            amount = LARGE if large else SMALL
            env.process(consumer(amount, large, arrival))

    def producer():
        for _ in range(n):
            yield env.timeout(PUT_INTERVAL)
            yield cont.put(PUT_AMT)

    env.process(arrivals())
    env.process(producer())
    env.run()

    def mean_of(which):
        xs = [w for (lg, w) in recs if which is None or lg == which]
        return sum(xs) / len(xs) if xs else 0.0

    return {
        "mean_wait_all": mean_of(None),
        "mean_wait_small": mean_of(False),
        "mean_wait_large": mean_of(True),
        "served": float(len(recs)),
    }


def events_per_seed(rec, args):
    return args.n + int(rec["served"])


if __name__ == "__main__":
    run("container", run_seed, events_per_seed)
