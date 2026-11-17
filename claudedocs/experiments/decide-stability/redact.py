"""Redact the decide-stability measurement files for committing (054, FR-010).

Usage: python -I -X utf8 redact.py <scratch folder>
Reads probe_out1.jsonl, probe_out2.jsonl, probe_plan1.json.
Writes redacted/results.jsonl and redacted/plan-redacted.json in the same folder.
Stops with an error rather than guess: a failed call that is not the provider's
billing error, a case id outside c00..c09, or an unexpected variant name.
"""
import io
import json
import os
import re
import sys

folder = sys.argv[1]
out_dir = os.path.join(folder, "redacted")
os.makedirs(out_dir, exist_ok=True)

IDS = {"c%02d" % i for i in range(10)}
VARIANT = re.compile(r"^(same\d+|order\d+|drop\d+|order\d+#\d+|drop\d+#\d+)$")
METHODS = {"weigh", "causal", "probabilistic"}
BILLING = "credit balance is too low"


def fail(msg):
    print("STOP:", msg)
    sys.exit(1)


lines_out = []
counts = {"attempted": 0, "completed": 0, "billing": 0}
for name in ("probe_out1.jsonl", "probe_out2.jsonl"):
    for n, line in enumerate(io.open(os.path.join(folder, name), encoding="utf-8"), 1):
        r = json.loads(line)
        where = "%s line %d" % (name, n)
        if r.get("id") not in IDS:
            fail("%s: unexpected case id %r" % (where, r.get("id")))
        if not isinstance(r.get("variant"), str) or not VARIANT.match(r["variant"]):
            fail("%s: unexpected variant %r" % (where, r.get("variant")))
        perm = r.get("perm")
        if not (isinstance(perm, list) and sorted(perm) == list(range(len(perm)))):
            fail("%s: perm is not a permutation: %r" % (where, perm))
        counts["attempted"] += 1
        row = {"id": r["id"], "variant": r["variant"], "perm": perm}
        if "error" in r:
            if BILLING not in r["error"]:
                fail("%s: a failure that is not the billing error: %s" % (where, r["error"][:200]))
            row["error"] = "billing"
            counts["billing"] += 1
        else:
            scores = r.get("scores")
            if not (isinstance(scores, list) and len(scores) == len(perm)
                    and all(isinstance(s, int) and 0 <= s <= 100 for s in scores)):
                fail("%s: unexpected scores %r" % (where, scores))
            if r.get("methodology") not in METHODS:
                fail("%s: unexpected methodology %r" % (where, r.get("methodology")))
            for k in ("input_tokens", "output_tokens"):
                if not isinstance(r.get(k), int):
                    fail("%s: %s is not an integer" % (where, k))
            row.update(scores=scores, methodology=r["methodology"],
                       input_tokens=r["input_tokens"], output_tokens=r["output_tokens"])
            counts["completed"] += 1
        lines_out.append(json.dumps(row, separators=(",", ":")))

plan = json.load(io.open(os.path.join(folder, "probe_plan1.json"), encoding="utf-8"))
cases = []
for c in plan["cases"]:
    if c.get("id") not in IDS:
        fail("plan: unexpected case id %r" % c.get("id"))
    cases.append({
        "id": c["id"],
        "options": len(c["options"]),
        "sentences": len(c["sentences"]),
        "recorded_scores": c["recorded_scores"],
        "recorded_gap": c["recorded_gap"],
    })
if sorted(c["id"] for c in cases) != sorted(IDS):
    fail("plan: the ten case ids are not exactly c00..c09")

io.open(os.path.join(out_dir, "results.jsonl"), "w", encoding="utf-8", newline="\n").write("\n".join(lines_out) + "\n")
io.open(os.path.join(out_dir, "plan-redacted.json"), "w", encoding="utf-8", newline="\n").write(
    json.dumps({"cases": cases}, indent=1) + "\n")
print("written", counts)

# Independent check of the outputs: every string value must be in the vocabulary.
ALLOWED = re.compile(r"^(c0\d|same\d+|order\d+(#\d+)?|drop\d+(#\d+)?|weigh|causal|probabilistic|billing)$")
KEYS = {"id", "variant", "perm", "error", "scores", "methodology", "input_tokens", "output_tokens",
        "cases", "options", "sentences", "recorded_scores", "recorded_gap"}
bad = []


def walk(v, where):
    if isinstance(v, dict):
        for k, x in v.items():
            if k not in KEYS:
                bad.append("%s: unexpected key %r" % (where, k))
            walk(x, where)
    elif isinstance(v, list):
        for x in v:
            walk(x, where)
    elif isinstance(v, str):
        if not ALLOWED.match(v):
            bad.append("%s: unexpected string %r" % (where, v[:60]))
    elif not isinstance(v, int) or isinstance(v, bool):
        bad.append("%s: unexpected value %r" % (where, v))


for n, line in enumerate(io.open(os.path.join(out_dir, "results.jsonl"), encoding="utf-8"), 1):
    walk(json.loads(line), "results.jsonl line %d" % n)
walk(json.load(io.open(os.path.join(out_dir, "plan-redacted.json"), encoding="utf-8")), "plan-redacted.json")
print("vocabulary check: %d problems" % len(bad))
for b in bad[:20]:
    print("  ", b)
sys.exit(1 if bad else 0)
