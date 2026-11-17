"""Print the measurement table of specs/054-decide-reliability/spec.md.

Usage, from the repository root:
    python -I -X utf8 claudedocs/experiments/decide-stability/table.py

Reads only results.jsonl and plan-redacted.json beside this file. Standard
library only. It makes no network or model calls.

Definitions used throughout:
- A run's scores are stored in the order the options were presented; `perm`
  gives, for each presented position, the option's index in the original
  order. Scores are mapped back to the original order before comparing.
- A run's winner is the first highest score in presented order, which is how
  `decide` breaks ties.
- "Identical runs" are the variants named same*: the input sent unchanged.
- An input's "usual winner" is the option most often on top across its
  identical runs; a "less common winner" is any other.
"""
import collections
import itertools
import json
import os
import re
import statistics

HERE = os.path.dirname(os.path.abspath(__file__))
rows = [json.loads(line) for line in open(os.path.join(HERE, "results.jsonl"), encoding="utf-8")]
plan = json.load(open(os.path.join(HERE, "plan-redacted.json"), encoding="utf-8"))

done = [r for r in rows if "error" not in r]
failed = [r for r in rows if "error" in r]
single = re.compile(r"^(order|drop)\d+$")
unused = [r for r in done if single.match(r["variant"])]
used = [r for r in done if not single.match(r["variant"])]

print("calls attempted: %d" % len(rows))
print("calls completed: %d" % len(done))
print("calls failed (billing): %d" % len(failed))
print("completed calls used in the table: %d" % len(used))
print("completed calls not used (first-stage single reorder/removal runs): %d" % len(unused))
print()


def original(r):
    out = [None] * len(r["perm"])
    for pos, idx in enumerate(r["perm"]):
        out[idx] = r["scores"][pos]
    return out


def winner(r):
    best = max(r["scores"])
    return r["perm"][r["scores"].index(best)]


def gap(r):
    top = sorted(r["scores"], reverse=True)
    return top[0] - top[1]


def same_index(r):
    return int(r["variant"][4:])


ids = sorted({r["id"] for r in rows})
same = {i: sorted((r for r in used if r["id"] == i and r["variant"].startswith("same")), key=same_index)
        for i in ids}
usual = {i: collections.Counter(winner(r) for r in same[i]).most_common(1)[0][0] for i in ids}

changed = [i for i in ids if len({winner(r) for r in same[i]}) > 1]
print("inputs whose top option changed between identical runs: %d of %d (%s)"
      % (len(changed), len(ids), ", ".join(changed)))
print("identical runs per input: %s"
      % ", ".join("%s=%d" % (i, len(same[i])) for i in ids))
print("distinct winners per input over its identical runs: %s"
      % ", ".join("%s=%d" % (i, len({winner(r) for r in same[i]})) for i in ids))

diffs = []
for i in ids:
    for a, b in itertools.combinations(same[i], 2):
        diffs += [abs(x - y) for x, y in zip(original(a), original(b))]
diffs.sort()
print("difference between two identical runs of one option's score: "
      "median %g, 90th percentile %g, max %g (%d pairs)"
      % (statistics.median(diffs), diffs[int(len(diffs) * 0.9)], diffs[-1], len(diffs)))
big = collections.Counter()
for i in ids:
    for a, b in itertools.combinations(same[i], 2):
        for k, (x, y) in enumerate(zip(original(a), original(b))):
            if abs(x - y) >= 40:
                big[(i, k)] += 1
print("differences of 40 or more, by (input, option index): %s" % dict(big))

widest = max(ids, key=lambda i: max(gap(r) for r in same[i] if winner(r) != usual[i]) if any(
    winner(r) != usual[i] for r in same[i]) else -1)
print("input %s, top-two gap over its identical runs in order: %s; winners: %s"
      % (widest, [gap(r) for r in same[widest]], [winner(r) for r in same[widest]]))
by_winner = collections.defaultdict(list)
for r in same[widest]:
    by_winner[winner(r)].append(gap(r))
print("input %s, largest lead for each option that won: %s"
      % (widest, {k: max(v) for k, v in sorted(by_winner.items())}))
print()

all_same = [r for i in ids for r in same[i]]
for label, test in (("gap of 15 or less", lambda g: g <= 15), ("gap over 15", lambda g: g > 15),
                    ("gap over 20", lambda g: g > 20), ("gap over 25", lambda g: g > 25),
                    ("gap over 30", lambda g: g > 30), ("gap over 38", lambda g: g > 38)):
    sel = [r for r in all_same if test(gap(r))]
    less = [r for r in sel if winner(r) != usual[r["id"]]]
    print("identical runs with a %s that named the less common winner: %d of %d"
          % (label, len(less), len(sel)))
print("less-common-winner runs by input: %s"
      % dict(collections.Counter(r["id"] for r in all_same if winner(r) != usual[r["id"]])))
print("less-common-winner runs with a gap over 15, as input and gap: %s"
      % sorted((r["id"], gap(r)) for r in all_same if winner(r) != usual[r["id"]] and gap(r) > 15))
print("wins per option over identical runs, by input: %s"
      % ", ".join("%s=%s" % (i, sorted(collections.Counter(winner(r) for r in same[i]).values(), reverse=True))
                  for i in ids))
print("distinct winners in the first 5 identical runs, by input: %s"
      % ", ".join("%s=%d" % (i, len({winner(r) for r in same[i][:5]})) for i in ids))
print()

buckets = [("8 or less", lambda g: g <= 8), ("9 to 16", lambda g: 9 <= g <= 16),
           ("17 to 38", lambda g: 17 <= g <= 38), ("over 38", lambda g: g > 38)]
for label, test in buckets:
    total = differ = 0
    inputs = set()
    for i in ids:
        for a in same[i]:
            if not test(gap(a)):
                continue
            for b in same[i]:
                if b is a:
                    continue
                total += 1
                differ += winner(a) != winner(b)
                inputs.add(i)
    pct = (" (%d%%)" % round(100 * differ / total)) if total else ""
    print("pairs of identical runs naming different winners, first run's gap %s: %d of %d%s, from %d inputs"
          % (label, differ, total, pct, len(inputs)))
print()

groups = collections.defaultdict(list)
for r in used:
    if "#" in r["variant"]:
        groups[(r["id"], r["variant"].split("#")[0])].append(r)

drops = {k: v for k, v in groups.items() if k[1].startswith("drop")}
full = {k: v for k, v in drops.items() if len(v) == 5}
short = {k: len(v) for k, v in drops.items() if len(v) != 5}
reversed_all = sorted(k for k, v in full.items() if all(winner(r) != usual[k[0]] for r in v))
print("sentence removals with 5 runs: %d; with fewer: %s" % (len(full), dict(sorted(short.items()))))
print("removals where all 5 runs named a winner other than the usual one: %d of %d (%s), in %d of %d inputs"
      % (len(reversed_all), len(full), ", ".join("%s %s" % k for k in reversed_all),
         len({k[0] for k in reversed_all}), len({k[0] for k in drops})))
print()

orders = {k: v for k, v in groups.items() if k[1].startswith("order")}
order_inputs = sorted({k[0] for k in orders})
distinct = sum(len({tuple(v[0]["perm"]) for k, v in orders.items() if k[0] == i}) for i in order_inputs)
opposite = []
for i in order_inputs:
    unanimous = {}
    for k, v in orders.items():
        if k[0] == i and len(v) == 5 and len({winner(r) for r in v}) == 1:
            unanimous[k[1]] = winner(v[0])
    if len(set(unanimous.values())) > 1:
        opposite.append("%s %s" % (i, unanimous))
print("distinct reorderings across %d inputs: %d" % (len(order_inputs), distinct))
print("reorder groups with fewer than 5 runs: %s"
      % dict(sorted((k, len(v)) for k, v in orders.items() if len(v) != 5)))
print("inputs where one ordering gave one winner 5 of 5 times and another ordering a different winner 5 of 5 times: "
      "%d of %d (%s)" % (len(opposite), len(order_inputs), "; ".join(opposite)))
print()

doubled = []
for i in ids:
    base = statistics.median(r["input_tokens"] for r in same[i])
    doubled += ["%s %s" % (i, r["variant"]) for r in same[i] if r["input_tokens"] > 1.5 * base]
print("identical runs with about twice their siblings' input tokens (a retry is inferred): %d of %d (%s)"
      % (len(doubled), len(all_same), ", ".join(doubled)))
print("recorded gaps of the planned cases, in plan order: %s"
      % [c["recorded_gap"] for c in plan["cases"]])
print("inputs with reorder/removal repeats: %s" % ", ".join(order_inputs))
