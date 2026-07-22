"""decide order-bias experiment runner.

Speaks MCP stdio (newline-delimited JSON-RPC) directly to the release binary.
Env comes from the user's parallax MCP config (~/.claude.json) and is passed
to the subprocess only — never printed. Each worker gets its own server
process and its own scratch DATABASE_PATH so the live DB is untouched.
"""

import json
import os
import pathlib
import queue
import subprocess
import sys
import threading

ROOT = pathlib.Path(__file__).resolve().parent
EXE = (
    ROOT.parents[2] / "target" / "release" / "mcp-parallax.exe"
)  # claudedocs/experiments/decide-order-bias -> repo root
SCRATCH = pathlib.Path(os.environ["EXP_SCRATCH"])  # required: scratch dir for DBs
RESULTS = ROOT / "results.jsonl"
WORKERS = 4

write_lock = threading.Lock()


def load_env() -> dict:
    cfg = json.load(open(pathlib.Path.home() / ".claude.json", encoding="utf-8"))
    env = dict(os.environ)
    env.update(cfg["mcpServers"]["parallax"]["env"])
    return env


class Server:
    def __init__(self, worker_id: int, env: dict):
        e = dict(env)
        SCRATCH.mkdir(parents=True, exist_ok=True)
        e["DATABASE_PATH"] = str(SCRATCH / f"order-bias-{worker_id}.db")
        e["LOG_LEVEL"] = "warn"
        self.model = e.get("ANTHROPIC_MODEL", "claude-opus-4-8")
        self.proc = subprocess.Popen(
            [str(EXE)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            env=e,
            text=True,
            encoding="utf-8",
            bufsize=1,
        )
        self.rid = 0
        self._request(
            "initialize",
            {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "order-bias-runner", "version": "0.1"},
            },
        )
        self._send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def _send(self, msg: dict) -> None:
        self.proc.stdin.write(json.dumps(msg) + "\n")
        self.proc.stdin.flush()

    def _request(self, method: str, params: dict) -> dict:
        self.rid += 1
        rid = self.rid
        self._send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
        while True:
            line = self.proc.stdout.readline()
            if not line:
                raise RuntimeError("server closed stdout")
            msg = json.loads(line)
            if msg.get("id") == rid:
                return msg

    def decide(self, decision: str, options: list, context: str) -> dict:
        return self._request(
            "tools/call",
            {
                "name": "decide",
                "arguments": {
                    "decision": decision,
                    "options": options,
                    "context": context,
                },
            },
        )

    def close(self) -> None:
        try:
            self.proc.stdin.close()
            self.proc.wait(timeout=10)
        except Exception:
            self.proc.kill()


def arms_for(group: str, n_options: int) -> list:
    identity = list(range(n_options))
    arms = [("orig1", identity), ("orig2", identity), ("rev", identity[::-1])]
    if group == "near4":
        arms.append(("rot1", identity[1:] + identity[:1]))
    return arms


def structured(resp: dict):
    result = resp.get("result") or {}
    sc = result.get("structuredContent")
    if sc is not None:
        return sc
    for block in result.get("content") or []:
        if block.get("type") == "text":
            try:
                return json.loads(block["text"])
            except (json.JSONDecodeError, KeyError):
                pass
    return None


def worker(worker_id: int, jobs: "queue.Queue", env: dict, out) -> None:
    server = Server(worker_id, env)
    try:
        while True:
            try:
                job = jobs.get_nowait()
            except queue.Empty:
                return
            fixture, group, arm, order = job
            options = [fixture["options"][i] for i in order]
            row = {
                "problem_id": fixture["id"],
                "group": group,
                "shape": fixture["shape"],
                "arm": arm,
                "order": order,
                "model": server.model,
            }
            try:
                resp = server.decide(
                    fixture["decision"], options, fixture.get("context", "")
                )
                if "error" in resp:
                    row["error"] = resp["error"].get("message", "unknown")
                else:
                    row["result"] = structured(resp)
                    if (resp.get("result") or {}).get("isError"):
                        row["error"] = "tool isError"
            except Exception as exc:  # record, keep going
                row["error"] = f"{type(exc).__name__}: {exc}"
            with write_lock:
                out.write(json.dumps(row) + "\n")
                out.flush()
                print(f"[{fixture['id']}/{arm}] {'ERR' if 'error' in row else 'ok'}")
            jobs.task_done()
    finally:
        server.close()


def main() -> None:
    fixtures = json.load(open(ROOT / "fixtures.json", encoding="utf-8"))
    env = load_env()
    jobs: "queue.Queue" = queue.Queue()
    total = 0
    for group, items in fixtures.items():
        for fixture in items:
            for arm, order in arms_for(group, len(fixture["options"])):
                jobs.put((fixture, group, arm, order))
                total += 1
    print(f"{total} calls across {WORKERS} workers")
    with open(RESULTS, "w", encoding="utf-8") as out:
        threads = [
            threading.Thread(target=worker, args=(i, jobs, env, out), daemon=True)
            for i in range(WORKERS)
        ]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
    print("done")


if __name__ == "__main__":
    sys.exit(main())
