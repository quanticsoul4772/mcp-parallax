"""Download the attack files for the Jev injection back-test (065 T028a).

Fetches only the files the dataset needs, from the public BIPIA and InjecAgent
repositories at pinned commits, into OUT_DIR/raw/. Nothing is executed from
them.

    python -I fetch_attacks.py OUT_DIR
"""

import pathlib
import sys
import urllib.request

BIPIA = "https://raw.githubusercontent.com/microsoft/BIPIA/a004b69ec0dd446e0afd461d98cb5e96e120a5d0/"
INJECAGENT = "https://raw.githubusercontent.com/uiuc-kang-lab/InjecAgent/f19c9f2c79a41046eb13c03c51a24c567a8ffa07/"

FILES = {
    "bipia_text_attack_test.json": BIPIA + "benchmark/text_attack_test.json",
    "injecagent_attacker_cases_dh.jsonl": INJECAGENT + "data/attacker_cases_dh.jsonl",
    "injecagent_attacker_cases_ds.jsonl": INJECAGENT + "data/attacker_cases_ds.jsonl",
    "injecagent_test_cases_dh_enhanced.json": INJECAGENT + "data/test_cases_dh_enhanced.json",
}


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit("usage: python -I fetch_attacks.py OUT_DIR")
    raw = pathlib.Path(sys.argv[1]) / "raw"
    raw.mkdir(parents=True, exist_ok=True)
    for name, url in FILES.items():
        with urllib.request.urlopen(url, timeout=60) as response:
            data = response.read()
        (raw / name).write_bytes(data)
        print(f"{name}: {len(data)} bytes")


if __name__ == "__main__":
    main()
