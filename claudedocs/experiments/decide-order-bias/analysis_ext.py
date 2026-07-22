"""Pooled four-option analysis: original near4 (n=10) + extension (n=30)."""

import json
import math
import pathlib
from collections import defaultdict

ROOT = pathlib.Path(__file__).resolve().parent


def binom_two_sided(k, n):
    if n == 0:
        return 1.0
    lo = min(k, n - k)
    p = sum(math.comb(n, i) for i in range(0, lo + 1)) / 2**n
    return min(1.0, 2 * p)


def load(path):
    rows = []
    for line in open(path, encoding="utf-8"):
        if line.strip():
            rows.append(json.loads(line))
    return rows


def main():
    rows = load(ROOT / "results.jsonl") + load(ROOT / "results-ext.jsonl")
    by_problem = defaultdict(dict)
    errors = 0
    for r in rows:
        if r["group"] != "near4":
            continue
        if "error" in r or not r.get("result"):
            errors += 1
            continue
        by_problem[r["problem_id"]][r["arm"]] = r
    print(f"four-option problems: {len(by_problem)}, call errors: {errors}")

    def winner(c):
        return c["result"]["recommended"]

    def margin(c):
        ss = sorted((a["score"] for a in c["result"]["assessments"]), reverse=True)
        return ss[0] - ss[1] if len(ss) >= 2 else 0

    n_retest = retest_flips = 0
    pair_total = pair_flips = 0
    b = c = 0
    per_problem = []
    for pid, calls in sorted(by_problem.items()):
        if "orig1" not in calls:
            continue
        w1 = winner(calls["orig1"])
        r_flip = "orig2" in calls and winner(calls["orig2"]) != w1
        if "orig2" in calls:
            n_retest += 1
            retest_flips += r_flip
        o_flip = False
        for perm in ("rev", "rot1"):
            if perm in calls:
                pair_total += 1
                if winner(calls[perm]) != w1:
                    pair_flips += 1
                    o_flip = True
        if o_flip and not r_flip:
            b += 1
        if r_flip and not o_flip:
            c += 1
        per_problem.append((pid, margin(calls["orig1"]), r_flip, o_flip))

    r0 = retest_flips / n_retest if n_retest else 0.0
    r1 = pair_flips / pair_total if pair_total else 0.0
    print(f"r0 (retest): {retest_flips}/{n_retest} = {r0:.1%}")
    print(f"r1 (permuted pairs): {pair_flips}/{pair_total} = {r1:.1%}")
    print(f"effect r1-r0: {r1 - r0:+.1%}")
    print(f"discordant problems: order-only={b}, retest-only={c}")
    print(f"exact sign test (two-sided): p = {binom_two_sided(b, b + c):.4f}")

    print("\nflips vs orig1 margin:")
    for lo, hi in [(0, 8), (9, 16), (17, 100)]:
        bucket = [t for t in per_problem if lo <= t[1] <= hi]
        if not bucket:
            continue
        print(
            f"  margin {lo}-{hi}: n={len(bucket)} "
            f"order-flips={sum(1 for t in bucket if t[3])} "
            f"retest-flips={sum(1 for t in bucket if t[2])}"
        )

    flipped = [t for t in per_problem if t[3]]
    if flipped:
        print(f"\nmax margin with an order flip: {max(t[1] for t in flipped)}")
    stable = [t for t in per_problem if not t[3] and not t[2]]
    if stable:
        low_stable = sorted(t[1] for t in stable)[:5]
        print(f"lowest margins that stayed fully stable: {low_stable}")


if __name__ == "__main__":
    main()
