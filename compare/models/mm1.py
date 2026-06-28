"""M/M/1 queue (single server). SimPy twin of `compare --model mm1`."""

from _common import run
from queue_model import run_queue_seed


def run_seed(seed, args):
    return run_queue_seed(seed, args.n, args.lam, args.mu, 1)


def events_per_seed(rec, args):
    return 2 * args.n  # one arrival + one departure transition per customer


if __name__ == "__main__":
    run("mm1", run_seed, events_per_seed)
