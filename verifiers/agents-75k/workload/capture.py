#!/usr/bin/env python3
"""Capture the coding-agent profile from coding sessions: numbers, never content.

  capture.py ~/.claude/projects            # every Claude Code transcript under it
  capture.py t1.jsonl t2.jsonl events.json # transcripts and list_events dumps
  capture.py --out workload/coding-profile.json ...

Reads Claude Code session transcripts (*.jsonl) and claude-code-remote
list_events `result` dumps (*.json, a list of result events or of
{created_at, duration_ms, num_turns, context_tokens, output_tokens}), and
writes the distributions (n, mean, median, p90, p99, max) of each shape the
1M goal's coding agents are built from.

Privacy: only counts, sizes, durations and gaps leave a transcript. No
prompt, answer, code, file name, command or tool argument is read into the
output, and tool names are folded into a few kinds. The output is reviewed
and committed like the goal file.
"""

import argparse
import datetime as dt
import glob
import json
import os
import statistics
import sys

TOOL_KINDS = {
    "Bash": "shell", "BashOutput": "shell",
    "Read": "read", "NotebookRead": "read", "WebFetch": "read",
    "Edit": "edit", "Write": "edit", "MultiEdit": "edit", "NotebookEdit": "edit",
    "Grep": "search", "Glob": "search", "WebSearch": "search", "ToolSearch": "search",
    "Agent": "subagent", "Task": "subagent",
}


def kind(name):
    if name.startswith("mcp__"):
        return "mcp"
    return TOOL_KINDS.get(name, "other")


def ts(s):
    return dt.datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()


def quantile(xs, q):
    xs = sorted(xs)
    if not xs:
        return None
    i = q * (len(xs) - 1)
    lo, hi = int(i), min(int(i) + 1, len(xs) - 1)
    return xs[lo] + (xs[hi] - xs[lo]) * (i - lo)


def dist(xs):
    xs = [x for x in xs if x is not None]
    if not xs:
        return {"n": 0}
    r = lambda v: round(v, 3)
    return {"n": len(xs), "mean": r(statistics.fmean(xs)), "median": r(quantile(xs, 0.5)),
            "p90": r(quantile(xs, 0.9)), "p99": r(quantile(xs, 0.99)), "max": r(max(xs))}


def is_prompt(row):
    """A turn starts at a person's (or a wake's) message, not a tool result."""
    if row.get("type") != "user" or row.get("isMeta") or row.get("isSidechain"):
        return False
    content = (row.get("message") or {}).get("content")
    if isinstance(content, str):
        return True
    return isinstance(content, list) and not any(
        isinstance(c, dict) and c.get("type") == "tool_result" for c in content)


def tool_result_bytes(row):
    out = []
    for c in (row.get("message") or {}).get("content") or []:
        if isinstance(c, dict) and c.get("type") == "tool_result":
            body = c.get("content")
            if isinstance(body, list):
                n = sum(len(x.get("text", "")) for x in body if isinstance(x, dict))
            else:
                n = len(body or "")
            out.append(n)
    return out


def transcript_turns(path):
    """Per-turn numbers from one Claude Code transcript."""
    rows = []
    with open(path, errors="replace") as f:
        for line in f:
            try:
                rows.append(json.loads(line))
            except ValueError:
                continue
    turns, cur, compactions, subagents = [], None, 0, 0
    for row in rows:
        t = row.get("timestamp")
        if row.get("type") == "system" and row.get("subtype") == "compact_boundary":
            compactions += 1
        if is_prompt(row) and t:
            cur = {"start": ts(t), "end": ts(t), "calls": set(), "context": [], "output": 0,
                   "tools": {}, "tool_kb": []}
            turns.append(cur)
            continue
        if cur is None or not t:
            continue
        if row.get("type") == "assistant" and not row.get("isSidechain"):
            cur["end"] = max(cur["end"], ts(t))
            msg = row.get("message") or {}
            mid = msg.get("id")
            u = msg.get("usage") or {}
            if mid and mid not in cur["calls"]:
                cur["calls"].add(mid)
                cur["context"].append(u.get("input_tokens", 0) + u.get("cache_read_input_tokens", 0)
                                      + u.get("cache_creation_input_tokens", 0))
                cur["output"] += u.get("output_tokens", 0)
            for c in msg.get("content") or []:
                if isinstance(c, dict) and c.get("type") == "tool_use":
                    k = kind(c.get("name", ""))
                    cur["tools"][k] = cur["tools"].get(k, 0) + 1
                    if k == "subagent":
                        subagents += 1
        elif row.get("type") == "user":
            if tool_result_bytes(row):
                cur["end"] = max(cur["end"], ts(t))
            cur["tool_kb"] += [b / 1024 for b in tool_result_bytes(row)]
    out = []
    for i, tn in enumerate(turns):
        if not tn["calls"]:
            continue
        nxt = turns[i + 1]["start"] if i + 1 < len(turns) else None
        out.append({
            "start": tn["start"],
            "seconds": tn["end"] - tn["start"],
            "model_calls": len(tn["calls"]),
            "context_tokens": tn["context"],
            "output_tokens": tn["output"],
            "tools": tn["tools"],
            "tool_kb": tn["tool_kb"],
            "gap_minutes": (nxt - tn["end"]) / 60 if nxt else None,
        })
    return out, {"compactions": compactions, "subagents": subagents}


def event_turns(path):
    """Per-turn numbers from a list_events dump (result events only)."""
    with open(path) as f:
        data = json.load(f)
    if isinstance(data, dict):
        data = (data.get("ccr") or data).get("data", [])
    turns = []
    for ev in data:
        r = ev.get("result")
        if r is not None:
            r = r.get("internal_anthropic_catchall", r)
            usage = r.get("usage") or {}
            its = usage.get("iterations") or []
            ctx = [i.get("input_tokens", 0) + i.get("cache_read_input_tokens", 0)
                   + i.get("cache_creation_input_tokens", 0) for i in its]
            row = {"created_at": ev["created_at"], "duration_ms": r.get("duration_ms"),
                   "num_turns": r.get("num_turns"), "context_tokens": ctx,
                   "output_tokens": usage.get("output_tokens")}
        else:
            row = ev
        end = ts(row["created_at"])
        secs = (row.get("duration_ms") or 0) / 1000
        ctx = row.get("context_tokens") or []
        turns.append({"start": end - secs, "end": end, "seconds": secs,
                      "model_calls": row.get("num_turns"),
                      "context_tokens": ctx if isinstance(ctx, list) else [ctx],
                      "output_tokens": row.get("output_tokens"), "tools": {}, "tool_kb": []})
    turns.sort(key=lambda t: t["start"])
    for i, tn in enumerate(turns):
        nxt = turns[i + 1]["start"] if i + 1 < len(turns) else None
        tn["gap_minutes"] = (nxt - tn["end"]) / 60 if nxt is not None else None
        del tn["end"]
    return turns, {}


def profile(sessions):
    """sessions: list of (turns, extras)."""
    turns = [t for s, _ in sessions for t in s]
    kinds = {}
    for t in turns:
        for k, v in t["tools"].items():
            kinds[k] = kinds.get(k, 0) + v
    tool_total = sum(kinds.values())
    with_tools = [t for t in turns if t["tools"]]
    hours = []
    for s, _ in sessions:
        if s:
            last = s[-1]
            hours.append((last["start"] + last["seconds"] - s[0]["start"]) / 3600)
    return {
        "source": {
            "sessions": len([s for s, _ in sessions if s]),
            "turns": len(turns),
            "note": "numbers only: counts, sizes, durations and gaps; no content",
        },
        "shapes": {
            "turns_per_session": dist([len(s) for s, _ in sessions if s]),
            "session_hours": dist(hours),
            "turn_seconds": dist([t["seconds"] for t in turns]),
            "gap_minutes": dist([t["gap_minutes"] for t in turns]),
            "model_calls_per_turn": dist([t["model_calls"] for t in turns]),
            "context_tokens_per_call": dist([c for t in turns for c in t["context_tokens"]]),
            "output_tokens_per_turn": dist([t["output_tokens"] for t in turns]),
            "tool_calls_per_turn": dist([sum(t["tools"].values()) for t in with_tools]),
            "tool_output_kb": dist([k for t in turns for k in t["tool_kb"]]),
            "subagents_per_session": dist([x.get("subagents", 0) for s, x in sessions if s and x]),
            "compactions_per_session": dist([x.get("compactions", 0) for s, x in sessions if s and x]),
        },
        "tool_kinds": {k: round(v / tool_total, 3) for k, v in sorted(kinds.items())} if tool_total else {},
    }


def inputs(paths):
    for p in paths:
        p = os.path.expanduser(p)
        if os.path.isdir(p):
            yield from sorted(glob.glob(os.path.join(p, "**", "*.jsonl"), recursive=True))
            yield from sorted(glob.glob(os.path.join(p, "**", "*.events.json"), recursive=True))
        else:
            yield p


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("paths", nargs="+")
    ap.add_argument("--out", help="write here instead of stdout")
    ap.add_argument("--min-turns", type=int, default=2, help="skip sessions shorter than this")
    a = ap.parse_args(argv)
    sessions = []
    for p in inputs(a.paths):
        turns, extra = event_turns(p) if p.endswith(".json") else transcript_turns(p)
        if len(turns) >= a.min_turns:
            sessions.append((turns, extra))
    prof = profile(sessions)
    prof["source"]["captured"] = dt.date.today().isoformat()
    text = json.dumps(prof, indent=1, sort_keys=True) + "\n"
    if a.out:
        with open(a.out, "w") as f:
            f.write(text)
    else:
        sys.stdout.write(text)
    print(f"{prof['source']['sessions']} sessions, {prof['source']['turns']} turns", file=sys.stderr)


if __name__ == "__main__":
    main()
