"""Analysis for the decide order-bias experiment (see design.md)."""

import json
import math
import pathlib
from collections import defaultdict

ROOT = pathlib.Path(__file__).resolve().parent


def load():
    rows = [
        json.loads(line)
        for line in open(ROOT / "results.jsonl", encoding="utf-8")
        if line.strip()
    ]
    by_problem = defaultdict(dict)
    errors = []
    for r in rows:
        if "error" in r or not r.get("result"):
            errors.append((r["problem_id"], r["arm"], r.get("error", "no result")))
            continue
        by_problem[r["problem_id"]][r["arm"]] = r
    return by_problem, errors, rows


def winner(call):
    return call["result"]["recommended"]


def scores_by_option(call):
    return {a["option"]: a["score"] for a in call["result"]["assessments"]}


def margin(call):
    ss = sorted((a["score"] for a in call["result"]["assessments"]), reverse=True)
    return ss[0] - ss[1] if len(ss) >= 2 else 0


def binom_two_sided(k, n):
    """Exact two-sided sign test P(X<=min(k,n-k) or X>=max(...)) under p=0.5."""
    if n == 0:
        return 1.0
    lo = min(k, n - k)
    p = sum(math.comb(n, i) for i in range(0, lo + 1)) / 2**n
    return min(1.0, 2 * p)


def analyze():
    by_problem, errors, rows = load()
    groups = defaultdict(list)
    for pid, calls in by_problem.items():
        g = next(r["group"] for r in calls.values())
        groups[g].append((pid, calls))

    print(f"calls parsed: {sum(len(c) for c in by_problem.values())}, errors: {len(errors)}")
    for e in errors:
        print("  ERROR", e)

    summary = {}
    for g, problems in sorted(groups.items()):
        retest_flips, order_flip_pairs, order_pair_total = 0, 0, 0
        n_retest = 0
        b = c = 0  # discordant: order-only flips vs retest-only flips
        flips_by_problem = {}
        for pid, calls in problems:
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
                    order_pair_total += 1
                    if winner(calls[perm]) != w1:
                        order_flip_pairs += 1
                        o_flip = True
            flips_by_problem[pid] = (r_flip, o_flip)
            if o_flip and not r_flip:
                b += 1
            if r_flip and not o_flip:
                c += 1
        r0 = retest_flips / n_retest if n_retest else 0.0
        r1 = order_flip_pairs / order_pair_total if order_pair_total else 0.0
        summary[g] = {
            "n": len(problems),
            "r0": r0,
            "r0_raw": f"{retest_flips}/{n_retest}",
            "r1": r1,
            "r1_raw": f"{order_flip_pairs}/{order_pair_total}",
            "effect": r1 - r0,
            "sign_test_p": binom_two_sided(b, b + c),
            "discordant": f"order-only={b}, retest-only={c}",
            "flips_by_problem": flips_by_problem,
        }

    # H3: positional boost (2-option groups): score of the same option when
    # listed first vs listed second, recovered via the fixtures' canonical
    # option order plus each arm's order field.
    pos_boost = defaultdict(list)
    calls_index = defaultdict(dict)
    for r in rows:
        if "result" in r and r.get("result"):
            calls_index[r["problem_id"]][r["arm"]] = r
    for pid, calls in calls_index.items():
        if "orig1" not in calls or "rev" not in calls:
            continue
        g = calls["orig1"]["group"]
        if len(calls["orig1"]["order"]) != 2:
            continue
        orig_assess = calls["orig1"]["result"]["assessments"]
        rev_assess = calls["rev"]["result"]["assessments"]
        so = {a["option"]: a["score"] for a in orig_assess}
        sr = {a["option"]: a["score"] for a in rev_assess}
        # orig order = [0,1] → options[0] listed first; rev = [1,0] → options[1] first.
        # We stored the permuted options list implicitly; recover option names by
        # position: in orig1, first-listed is the option that appears in the
        # fixtures first — identify via rev: both dicts share the same two names.
        names = list(so.keys())
        if set(names) != set(sr.keys()) or len(names) != 2:
            continue
        # Determine which name was first in orig1: fixtures.json order.
        fx = FIXTURES_BY_ID[pid]
        first, second = fx["options"][0], fx["options"][1]
        if first not in so or second not in so:
            continue
        pos_boost[g].append(so[first] - sr[first])   # first: pos1 in orig, pos2 in rev
        pos_boost[g].append(sr[second] - so[second]) # second: pos1 in rev, pos2 in orig
    for g, vals in pos_boost.items():
        if not vals:
            continue
        mean = sum(vals) / len(vals)
        pos = sum(v > 0 for v in vals)
        neg = sum(v < 0 for v in vals)
        summary[g]["positional_boost_mean"] = round(mean, 2)
        summary[g]["positional_boost_sign"] = f"+{pos}/-{neg}/0:{len(vals)-pos-neg}"
        summary[g]["positional_boost_p"] = round(binom_two_sided(pos, pos + neg), 4)

    # H2: flips vs margin (near groups, margin from orig1)
    margin_rows = []
    for g in ("near2", "near4"):
        for pid, calls in groups.get(g, []):
            if "orig1" not in calls:
                continue
            m = margin(calls["orig1"])
            conf = calls["orig1"]["result"]["confidence"]
            r_flip, o_flip = summary[g]["flips_by_problem"][pid]
            margin_rows.append((pid, g, m, conf, r_flip, o_flip))
    margin_rows.sort(key=lambda t: t[2])
    n = len(margin_rows)
    terciles = [margin_rows[: n // 3], margin_rows[n // 3 : 2 * n // 3], margin_rows[2 * n // 3 :]]

    print("\n=== per-group summary ===")
    for g, s in sorted(summary.items()):
        keep = {k: v for k, v in s.items() if k != "flips_by_problem"}
        print(g, json.dumps(keep, indent=2))

    print("\n=== flips vs margin (near-tie problems, terciles by orig1 margin) ===")
    for i, t in enumerate(terciles):
        if not t:
            continue
        lo, hi = t[0][2], t[-1][2]
        oflips = sum(1 for r in t if r[5])
        rflips = sum(1 for r in t if r[4])
        print(
            f"tercile {i+1} (margin {lo}–{hi}): n={len(t)} "
            f"order-flips={oflips} retest-flips={rflips}"
        )

    print("\n=== confidence vs stability (near-tie) ===")
    stable = [r[3] for r in margin_rows if not r[5]]
    flipped = [r[3] for r in margin_rows if r[5]]
    if stable:
        print(f"mean confidence, order-stable problems:  {sum(stable)/len(stable):.3f} (n={len(stable)})")
    if flipped:
        print(f"mean confidence, order-flipped problems: {sum(flipped)/len(flipped):.3f} (n={len(flipped)})")

    print("\n=== flipped problems (near groups) ===")
    for g in ("near2", "near4", "dominated"):
        for pid, (r_flip, o_flip) in summary.get(g, {}).get("flips_by_problem", {}).items():
            if o_flip or r_flip:
                print(f"  {pid}: retest_flip={r_flip} order_flip={o_flip}")


FIXTURES = json.load(open(ROOT / "fixtures.json", encoding="utf-8"))
FIXTURES_BY_ID = {f["id"]: f for g in FIXTURES.values() for f in g}

if __name__ == "__main__":
    analyze()
