"""Performance comparison: wall-clock, events/sec, and peak memory.

Each tool is launched as a subprocess so memory is measured identically via
`getrusage(RUSAGE_CHILDREN).ru_maxrss`. Wall time is the in-process simulation
time the tool self-reports (`wall_secs`), which excludes interpreter/binary
startup so events/sec reflects the engine rather than process spawn. Peak RSS is
measured around the whole child process (startup included) and reported as-is.
"""

import contract


def bench_one(model, n, lam, mu, servers):
    # seeds=1: one large replication isolates engine throughput.
    simu = contract.run_simu(model, 1, n, lam, mu, servers)
    simpy = contract.run_simpy(model, 1, n, lam, mu, servers)

    def stats(r):
        wall = r["wall_secs"]
        return {
            "wall_secs": wall,
            "events": r["events"],
            "events_per_sec": (r["events"] / wall) if wall > 0 else 0.0,
            "peak_mb": r["_peak_mb"],
        }

    return {"model": model, "n": n, "simu": stats(simu), "simpy": stats(simpy)}


def default_config(scale=1.0):
    base = max(int(100_000 * scale), 2_000)
    return [
        dict(model="mm1", n=base, lam=0.9, mu=1.0, servers=1),
        dict(model="mmc", n=base, lam=1.6, mu=1.0, servers=2),
        dict(model="priority", n=base, lam=0.9, mu=1.0, servers=1),
    ]


def run_all(scale=1.0):
    return [bench_one(**cfg) for cfg in default_config(scale)]


def render_markdown(results):
    lines = ["## Performance: simu vs. SimPy", ""]
    lines.append(
        "| model | arrivals | tool | wall (s) | events/sec | peak RSS (MB) |"
    )
    lines.append("|---|---|---|---|---|---|")
    speedups = []
    for r in results:
        for tool in ("simu", "simpy"):
            s = r[tool]
            lines.append(
                f"| {r['model']} | {r['n']:,} | {tool} | {s['wall_secs']:.3f} "
                f"| {s['events_per_sec']:,.0f} | {s['peak_mb']:.1f} |"
            )
        sp = (r["simpy"]["wall_secs"] / r["simu"]["wall_secs"]
              if r["simu"]["wall_secs"] > 0 else float("nan"))
        mem = (r["simpy"]["peak_mb"] / r["simu"]["peak_mb"]
               if r["simu"]["peak_mb"] > 0 else float("nan"))
        speedups.append((r["model"], sp, mem))
    lines.append("")
    lines.append("| model | simu speedup × | simu memory advantage × |")
    lines.append("|---|---|---|")
    for model, sp, mem in speedups:
        lines.append(f"| {model} | {sp:.1f}× | {mem:.1f}× |")
    lines.append("")
    return "\n".join(lines)


def main():
    import argparse
    p = argparse.ArgumentParser()
    p.add_argument("--scale", type=float, default=1.0)
    a = p.parse_args()
    print(render_markdown(run_all(scale=a.scale)))


if __name__ == "__main__":
    main()
