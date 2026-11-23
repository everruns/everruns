#!/usr/bin/env python3
"""Trim saved 75k run artifacts into the backtest fixtures (numbers only).

  tests/make_fixtures.py <dir with one folder per saas run id>

Keeps summary.json (first_errors cut to their count), chaos.log and the
finish time for every run, and for the newest passing run a thinned
samples.jsonl (every 4th sample, only the fields the importer reads).
"""

import json
import os
import shutil
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
RUNS = ["43774871232", "43803384547", "43827766384", "44355318472", "44396952768", "45179434646", "45197793040"]
WITH_SAMPLES = "45197793040"
FIELDS = ("node", "leader", "rss_mb", "resident_mb", "log_mb", "sessions", "unshipped_ms")


def main(src):
    for r in RUNS:
        dst = os.path.join(HERE, "fixtures", "runs", r)
        os.makedirs(dst, exist_ok=True)
        s = json.load(open(os.path.join(src, r, "summary.json")))
        s["first_errors"] = [f"({len(s.get('first_errors') or [])} errors, text removed)"] if s.get("first_errors") else []
        json.dump(s, open(os.path.join(dst, "summary.json"), "w"), indent=0, sort_keys=True)
        for f in ("chaos.log", "finished"):
            if os.path.exists(os.path.join(src, r, f)):
                shutil.copy(os.path.join(src, r, f), dst)
        if r == WITH_SAMPLES:
            with open(os.path.join(src, r, "samples.jsonl")) as fi, open(os.path.join(dst, "samples.jsonl"), "w") as fo:
                for i, line in enumerate(fi):
                    if i % 4:
                        continue
                    row = json.loads(line)
                    key = "shards" if "shards" in row else "cells"
                    thin = {sh: [[n, {k: v.get(k) for k in FIELDS}] for n, v in ns] for sh, ns in row[key].items()}
                    fo.write(json.dumps({"t": row["t"], "shards": thin}, separators=(",", ":")) + "\n")


if __name__ == "__main__":
    main(sys.argv[1])
