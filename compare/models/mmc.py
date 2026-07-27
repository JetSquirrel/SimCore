# SPDX-FileCopyrightText: 2026 Christoph Kuhmuench <christoph.kuhmuench@gmail.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

"""M/M/c queue (`--servers c`). SimPy twin of `compare --model mmc`."""

from _common import run
from queue_model import run_queue_seed


def run_seed(seed, args):
    return run_queue_seed(seed, args.n, args.lam, args.mu, args.servers)


def events_per_seed(rec, args):
    return 2 * args.n


if __name__ == "__main__":
    run("mmc", run_seed, events_per_seed)
