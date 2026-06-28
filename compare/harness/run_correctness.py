"""Correctness comparison: are simu and SimPy statistically indistinguishable?

For each model we run R replications in both tools and compare the *distribution*
of every output metric across seeds (not seed-by-seed — the two tools draw
different random streams, so only the distributions are expected to agree).

A metric FAILS only when the difference is both **statistically significant**
(Welch's t-test p < alpha) **and material** (relative mean difference > tol).
That "significant AND material" gate avoids the large-sample trap where a
negligible difference becomes significant purely because R is large.
"""

import math

from scipy import stats

import contract


def _summary(xs):
    n = len(xs)
    mean = sum(xs) / n if n else 0.0
    sem = (stats.tstd(xs) / math.sqrt(n)) if n > 1 else 0.0
    return mean, sem


def compare_metric(a, b, tol, alpha):
    """Compare metric arrays `a` (simu) and `b` (simpy)."""
    mean_a, sem_a = _summary(a)
    mean_b, sem_b = _summary(b)
    denom = max(abs(mean_a), abs(mean_b), 1e-12)
    rel_diff = abs(mean_a - mean_b) / denom

    # Welch t-test on means; KS on the whole distribution (informational).
    if len(set(a)) <= 1 and len(set(b)) <= 1:
        t_p = 1.0 if mean_a == mean_b else 0.0
    else:
        t_p = float(stats.ttest_ind(a, b, equal_var=False).pvalue)
    ks_p = float(stats.ks_2samp(a, b).pvalue)

    significant = t_p < alpha
    material = rel_diff > tol
    passed = not (significant and material)
    return {
        "mean_simu": mean_a, "sem_simu": sem_a,
        "mean_simpy": mean_b, "sem_simpy": sem_b,
        "rel_diff": rel_diff, "t_p": t_p, "ks_p": ks_p,
        "passed": passed,
    }


def compare_model(model, seeds, n, lam, mu, servers, tol, alpha):
    simu = contract.run_simu(model, seeds, n, lam, mu, servers)
    simpy = contract.run_simpy(model, seeds, n, lam, mu, servers)
    keys = contract.metric_keys(simu)
    rows = []
    for key in keys:
        a = contract.column(simu, key)
        b = contract.column(simpy, key)
        row = compare_metric(a, b, tol, alpha)
        row["metric"] = key
        rows.append(row)
    return {
        "model": model, "seeds": seeds, "n": n,
        "lam": lam, "mu": mu, "servers": servers,
        "rows": rows,
        "all_passed": all(r["passed"] for r in rows),
    }


# Closed-form M/M/c mean wait in queue (Erlang C). Used as an extra ground-truth
# check that *both* tools land near theory, independent of each other.
def erlang_c_wq(lam, mu, c):
    a = lam / mu
    rho = a / c
    if rho >= 1.0:
        return float("nan")
    s = sum(a ** k / math.factorial(k) for k in range(c))
    last = a ** c / (math.factorial(c) * (1 - rho))
    p_wait = last / (s + last)
    return p_wait / (c * mu - lam)


def default_config(scale=1.0):
    s = max(int(200 * scale), 20)
    n = max(int(1000 * scale), 200)
    return [
        dict(model="mm1", seeds=s, n=n, lam=0.85, mu=1.0, servers=1),
        dict(model="mmc", seeds=s, n=n, lam=1.6, mu=1.0, servers=2),
        dict(model="priority", seeds=s, n=n, lam=0.85, mu=1.0, servers=1),
        dict(model="container", seeds=s, n=n, lam=0.9, mu=1.0, servers=1),
        dict(model="hospital", seeds=s, n=n, lam=0.9, mu=1.0, servers=1),
    ]


def run_all(scale=1.0, tol=0.05, alpha=0.01):
    results = []
    for cfg in default_config(scale):
        results.append(compare_model(tol=tol, alpha=alpha, **cfg))
    return results


def render_markdown(results):
    lines = ["## Correctness: simu vs. SimPy (distribution equivalence)", ""]
    lines.append(
        "FAIL = statistically significant (t-test p < 0.01) **and** material "
        "(relative mean difference > 5%).\n"
    )
    for res in results:
        status = "✅ PASS" if res["all_passed"] else "❌ FAIL"
        lines.append(
            f"### {res['model']}  ({status}) — "
            f"{res['seeds']} seeds × {res['n']} arrivals, "
            f"λ={res['lam']}, μ={res['mu']}, c={res['servers']}"
        )
        lines.append("")
        lines.append(
            "| metric | simu mean | simpy mean | rel.diff | t-test p | KS p | result |"
        )
        lines.append("|---|---|---|---|---|---|---|")
        for r in res["rows"]:
            mark = "pass" if r["passed"] else "**FAIL**"
            lines.append(
                f"| {r['metric']} | {r['mean_simu']:.4g} ± {r['sem_simu']:.2g} "
                f"| {r['mean_simpy']:.4g} ± {r['sem_simpy']:.2g} "
                f"| {r['rel_diff']*100:.2f}% | {r['t_p']:.3f} | {r['ks_p']:.3f} | {mark} |"
            )
        # Analytical ground-truth note for queue models.
        if res["model"] in ("mm1", "mmc"):
            wq = erlang_c_wq(res["lam"], res["mu"], res["servers"])
            simu_wq = next(r["mean_simu"] for r in res["rows"] if r["metric"] == "mean_wait")
            simpy_wq = next(r["mean_simpy"] for r in res["rows"] if r["metric"] == "mean_wait")
            lines.append("")
            lines.append(
                f"_Analytical mean wait (Erlang C) = **{wq:.3f}**; "
                f"simu = {simu_wq:.3f}, simpy = {simpy_wq:.3f}._"
            )
        lines.append("")
    return "\n".join(lines)


def main():
    import argparse
    p = argparse.ArgumentParser()
    p.add_argument("--scale", type=float, default=1.0)
    p.add_argument("--tol", type=float, default=0.05)
    p.add_argument("--alpha", type=float, default=0.01)
    a = p.parse_args()
    results = run_all(scale=a.scale, tol=a.tol, alpha=a.alpha)
    print(render_markdown(results))
    if not all(r["all_passed"] for r in results):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
