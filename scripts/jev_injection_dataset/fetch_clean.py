"""Fetch clean pages through Exa for the Jev injection back-test (065 T028a).

Runs each query in queries.json through Exa search with page text, keeps pages
of more than 1,500 characters (capped at 6,000), skips repeated addresses, and
stops a set once it has its target count. Writes OUT_DIR/clean.jsonl lines
{"text", "url", "set"}. Reads EXA_API_KEY from the environment and never
prints it.

    python -I fetch_clean.py OUT_DIR
"""

import json
import os
import pathlib
import sys
import urllib.request

HERE = pathlib.Path(__file__).resolve().parent
MIN_CHARS = 1500
MAX_CHARS = 6000
PER_QUERY = 10


def search(key: str, query: str) -> list:
    body = json.dumps(
        {
            "query": query,
            "numResults": PER_QUERY,
            "contents": {"text": {"maxCharacters": MAX_CHARS}},
        }
    ).encode()
    request = urllib.request.Request(
        "https://api.exa.ai/search",
        data=body,
        headers={"x-api-key": key, "content-type": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=60) as response:
        return json.load(response).get("results", [])


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit("usage: python -I fetch_clean.py OUT_DIR")
    key = os.environ.get("EXA_API_KEY", "").strip()
    if not key:
        sys.exit("EXA_API_KEY is not set")
    out = pathlib.Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    plan = json.loads((HERE / "queries.json").read_text(encoding="utf-8"))
    seen = set()
    searches = 0
    with (out / "clean.jsonl").open("w", encoding="utf-8") as sink:
        for set_name, spec in plan.items():
            kept = 0
            for query in spec["queries"]:
                if kept >= spec["target"]:
                    break
                searches += 1
                for hit in search(key, query):
                    text = (hit.get("text") or "")[:MAX_CHARS]
                    url = hit.get("url") or ""
                    if len(text) <= MIN_CHARS or not url or url in seen:
                        continue
                    seen.add(url)
                    sink.write(json.dumps({"text": text, "url": url, "set": set_name}) + "\n")
                    kept += 1
                    if kept >= spec["target"]:
                        break
            print(f"{set_name}: {kept} of {spec['target']} pages")
    print(f"searches: {searches}")


if __name__ == "__main__":
    main()
