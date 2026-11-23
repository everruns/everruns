"""The committed climbing record and git helpers.

results.jsonl  one line per recorded run (numbers only, about 8 KB)
attempts.jsonl one line per climbing attempt: idea, points, before, after, kept, why
checkpoints/   frozen per-point state under a name, for --against

Raw evidence (driver reports, samples, heap profiles) never goes here: it
stays in target/validators/agents-1m/<time>/ or the workflow's artifact.
"""

import datetime as dt
import hashlib
import json
import os
import subprocess

from . import config as C

RESULTS = os.path.join(C.ROOT, "results.jsonl")
ATTEMPTS = os.path.join(C.ROOT, "attempts.jsonl")
CHECKPOINTS = os.path.join(C.ROOT, "checkpoints")
EVIDENCE = os.path.join(C.REPO, "target", "validators", "agents-1m")
COMPACT_AFTER_DAYS = 90


def read_jsonl(path):
    if not os.path.exists(path):
        return []
    with open(path) as f:
        return [json.loads(line) for line in f if line.strip()]


def append_jsonl(path, row):
    with open(path, "a") as f:
        f.write(json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n")


def git(*args, cwd=C.REPO):
    r = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True)
    return r.stdout.strip() if r.returncode == 0 else None


def head_commit():
    return git("rev-parse", "HEAD")


def dirty(paths):
    out = git("status", "--porcelain", "--", *paths)
    return bool(out)


def changed_since(commit, paths):
    """True if any watched path differs between commit and the working tree."""
    if not paths:
        return False
    if git("cat-file", "-e", f"{commit}^{{commit}}") is None:
        return True
    r = subprocess.run(["git", "diff", "--quiet", commit, "--", *paths], cwd=C.REPO)
    return r.returncode != 0


def blob_sha(path):
    """git's blob id of a file's bytes, without asking git."""
    with open(path, "rb") as f:
        data = f.read()
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def watch_hash(paths, commit=None):
    """A digest of the watched files' contents: at a commit (git ls-tree), or
    in the working tree. Survives squash merges and rebases, unlike commit
    ids, so evidence stays fresh until the code it depends on changes."""
    if not paths:
        return "none"
    pairs = []
    if commit:
        out = git("ls-tree", "-r", commit, "--", *paths)
        if out is None:
            return None
        for line in out.splitlines():
            meta, path = line.split("\t", 1)
            pairs.append((path, meta.split()[2]))
    else:
        out = git("ls-files", "-co", "--exclude-standard", "--", *paths)
        if out is None:
            return None
        for path in out.splitlines():
            full = os.path.join(C.REPO, path)
            if os.path.isfile(full) and "__pycache__" not in path:
                pairs.append((path, blob_sha(full)))
    body = "\n".join(f"{p} {s}" for p, s in sorted(pairs))
    return hashlib.sha256(body.encode()).hexdigest()[:16]


def compact(rows, now=None):
    """Fold runs older than 90 days: keep each one's headline, drop its
    entries unless it still holds some point's newest evidence at a level."""
    now = now or dt.datetime.now(dt.timezone.utc)
    newest = {}
    for r in rows:
        for e in r.get("entries", []):
            newest[(e["point"], e["level"])] = r["run"]
    keep_ids = set(newest.values())
    out = []
    for r in rows:
        age = (now - dt.datetime.fromisoformat(r["at"].replace("Z", "+00:00"))).days
        if age > COMPACT_AFTER_DAYS and r["run"] not in keep_ids:
            r = {k: v for k, v in r.items() if k not in ("entries", "metrics")}
            r["compacted"] = True
        out.append(r)
    return out


def write_jsonl(path, rows):
    with open(path, "w") as f:
        for r in rows:
            f.write(json.dumps(r, sort_keys=True, separators=(",", ":")) + "\n")


def checkpoint_path(name):
    safe = "".join(ch for ch in name if ch.isalnum() or ch in "-_.")
    if not safe:
        raise ValueError("checkpoint names use letters, digits, - _ .")
    return os.path.join(CHECKPOINTS, safe + ".json")
