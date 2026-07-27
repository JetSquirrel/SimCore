# SPDX-FileCopyrightText: 2026 Christoph Kuhmuench <christoph.kuhmuench@gmail.com>
#
# SPDX-License-Identifier: MIT OR Apache-2.0

"""Shared CLI parsing, JSON emission, and the per-seed driver loop.

Every model script emits the *same* JSON contract that the Rust runner
(`examples/compare.rs`) emits, so the harness can compare them directly:

    {"tool":"simpy","model":"mm1","seeds":1000,"n":1000,"lambda":0.9,"mu":1.0,
     "servers":1,"events":2000000,"wall_secs":0.12,"per_seed":[{...}, ...]}
"""

import argparse
import json
import sys
import time


def parse_args():
    p = argparse.ArgumentParser()
    p.add_argument("--model")  # informational; each script hard-codes its name
    p.add_argument("--seeds", type=int, default=1)
    p.add_argument("--n", type=int, default=1000)
    p.add_argument("--lambda", dest="lam", type=float, default=0.9)
    p.add_argument("--mu", type=float, default=1.0)
    p.add_argument("--servers", type=int, default=1)
    return p.parse_args()


def run(model, run_seed, events_per_seed):
    """Drive `run_seed` across `--seeds` replications and emit the JSON result.

    `run_seed(seed, args) -> dict`     produces one per-seed metrics object.
    `events_per_seed(rec, args) -> int` counts simulated events for that seed,
                                        using a definition identical to Rust's.
    """
    args = parse_args()
    per_seed = []
    events = 0
    t0 = time.perf_counter()
    for seed in range(args.seeds):
        rec = run_seed(seed, args)
        per_seed.append(rec)
        events += events_per_seed(rec, args)
    wall_secs = time.perf_counter() - t0

    out = {
        "tool": "simpy",
        "model": model,
        "seeds": args.seeds,
        "n": args.n,
        "lambda": args.lam,
        "mu": args.mu,
        "servers": args.servers,
        "events": events,
        "wall_secs": wall_secs,
        "per_seed": per_seed,
    }
    json.dump(out, sys.stdout, separators=(",", ":"))
    sys.stdout.write("\n")
