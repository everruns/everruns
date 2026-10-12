"""End-of-run assertions and metrics.

Every assertion reads the session's persisted events through
`GET /v1/sessions/{id}/events` (paged with `after_sequence`), never the SSE
copy the driver used, and the session status through `GET /v1/sessions/{id}`.
The one check the API cannot answer, "did a session ever run on two workers at
once", reads the worker logs (see `worker_overlap`).
"""

from __future__ import annotations

import datetime as dt
import os
import re
import statistics
import subprocess
from collections import defaultdict

from load import TERMINAL, turn_id_of

# Events that legitimately name a turn after its terminal event: session-level
# lifecycle (`session.idled` carries the turn it idled after).
AFTER_TERMINAL_OK = {"session.idled", "session.activated", "session.title.updated"}


def is_cancel_note(e, term) -> bool:
    """The cancel handler records "User requested to cancel the work." as an
    input.message inside the cancelled turn, right after `turn.cancelled`.
    That is the transcript note of the cancel, not work after the end."""
    if term["event_type"] != "turn.cancelled" or e["event_type"] != "input.message":
        return False
    msg = (e.get("data") or {}).get("message") or {}
    text = "".join(p.get("text", "") for p in msg.get("content") or [] if isinstance(p, dict))
    return "requested to cancel" in text


def parse_ts(s: str) -> float:
    s = s.replace("Z", "+00:00")
    # Python <3.11 cannot parse nanoseconds; trim to micros.
    m = re.match(r"(.*\.\d{6})\d*(.*)", s)
    if m:
        s = m.group(1) + m.group(2)
    return dt.datetime.fromisoformat(s).timestamp()


def pct(values, p):
    if not values:
        return None
    v = sorted(values)
    k = (len(v) - 1) * p / 100
    f, c = int(k), min(int(k) + 1, len(v) - 1)
    return v[f] + (v[c] - v[f]) * (k - f)


class Report:
    def __init__(self):
        self.failures: dict[str, list[str]] = defaultdict(list)
        self.counts: dict[str, int] = defaultdict(int)
        self.notes: list[str] = []

    def fail(self, check: str, session_id: str, detail: str):
        self.failures[check].append(f"{session_id}: {detail}")


def message_text(e):
    d = e.get("data") or {}
    msg = d.get("message") or d
    parts = msg.get("content") or []
    if isinstance(parts, str):
        return parts
    return "".join(p.get("text", "") for p in parts if isinstance(p, dict))


def check_session(run, events: list[dict], session: dict | None, rep: Report, timings: dict):
    sid = run.session_id
    by_type = defaultdict(list)
    for e in events:
        by_type[e["event_type"]].append(e)

    # Turns and their terminal events.
    started = {}
    started_by_msg = defaultdict(list)
    for e in by_type["turn.started"]:
        tid = turn_id_of(e)
        if tid in started:
            # Keep the first: a replayed turn.started (same turn id) must not
            # move the turn's start past inputs steered into it.
            rep.fail("turn_started_twice", sid, f"turn {tid} at seq {started[tid].get('sequence')} and {e.get('sequence')}")
            continue
        started[tid] = e
        started_by_msg[(e.get("data") or {}).get("input_message_id")].append(tid)
    terminals = defaultdict(list)
    for t in TERMINAL:
        for e in by_type[t]:
            terminals[turn_id_of(e)].append(e)

    # Inputs (no turn id of their own) that arrived while a turn was open.
    steered_into = defaultdict(int)
    for e in by_type["input.message"]:
        if (e.get("context") or {}).get("turn_id"):
            continue
        mid_ = ((e.get("data") or {}).get("message") or {}).get("id")
        if mid_ in started_by_msg:
            continue
        seq_ = e.get("sequence") or 0
        for tid_, s_ in started.items():
            end_ = min((t.get("sequence") or 0 for t in terminals.get(tid_, [])), default=10**12)
            if (s_.get("sequence") or 0) < seq_ < end_:
                steered_into[tid_] += 1

    rep.counts["turns_started"] += len(started)
    for tid, s in started.items():
        terms = terminals.get(tid, [])
        if not terms:
            rep.fail("turn_not_terminal", sid, f"turn {tid} started seq {s.get('sequence')} has no terminal event")
            continue
        if len(terms) > 1:
            rep.fail(
                "turn_two_terminals",
                sid,
                f"turn {tid}: " + ", ".join(f"{t['event_type']}@{t.get('sequence')}" for t in terms),
            )
        term = min(terms, key=lambda e: e.get("sequence") or 0)
        rep.counts[term["event_type"]] += 1
        tseq = term.get("sequence") or 0
        late = [
            f"{e['event_type']}@{e.get('sequence')}"
            for e in events
            if turn_id_of(e) == tid
            and (e.get("sequence") or 0) > tseq
            and e["event_type"] not in AFTER_TERMINAL_OK
            and not is_cancel_note(e, term)
        ]
        if late:
            rep.fail("event_after_terminal", sid, f"turn {tid} {term['event_type']}@{tseq} then {late[:5]}")
        # Duplicate assistant output inside one turn.
        outs = [e for e in by_type["output.message.completed"] if turn_id_of(e) == tid]
        ids = [((e.get("data") or {}).get("message") or {}).get("id") for e in outs]
        dup_ids = {i for i in ids if i and ids.count(i) > 1}
        if dup_ids:
            rep.fail("duplicate_output", sid, f"turn {tid} output message ids repeated {sorted(dup_ids)}")
        def is_final(e):
            msg = (e.get("data") or {}).get("message") or {}
            if msg.get("phase"):
                return msg["phase"] == "final_answer"
            return not msg.get("tool_calls")

        finals = [e for e in outs if is_final(e)]
        # A steered message is answered inside the turn it joined, so a turn
        # may end with one final answer per input it took.
        allowed = 1 + steered_into.get(tid, 0)
        if len(finals) > allowed:
            texts = [message_text(e)[:40] for e in finals]
            rep.fail("duplicate_output", sid, f"turn {tid} has {len(finals)} final answers {texts}")
        if term["event_type"] == "turn.completed" and not outs:
            rep.fail("completed_without_output", sid, f"turn {tid}")
        timings["server_ms"].append((parse_ts(term["ts"]) - parse_ts(s["ts"])) * 1000)
        timings["turn_spans"].append((parse_ts(s["ts"]), parse_ts(term["ts"]), sid, tid, term["event_type"]))

    # Messages the driver sent. A POST that died with its server may still
    # have been persisted: recover it by client_message_id from input.message.
    inputs = by_type["input.message"]
    by_cmid = {}
    for e in inputs:
        blob = repr(e.get("data"))
        for m in run.messages:
            if m.client_message_id in blob:
                by_cmid[m.client_message_id] = e
    for m in run.messages:
        rep.counts["messages_sent"] += 1
        ev = by_cmid.get(m.client_message_id)
        mid = m.id
        if ev is not None:
            ev_id = ((ev.get("data") or {}).get("message") or {}).get("id") or (ev.get("context") or {}).get(
                "input_message_id"
            )
            mid = mid or ev_id
        if not mid:
            rep.counts["messages_not_accepted"] += 1
            continue
        same = [e for e in inputs if ((e.get("data") or {}).get("message") or {}).get("id") == mid]
        if len(same) > 1:
            rep.fail("input_persisted_twice", sid, f"message {mid} has {len(same)} input.message events")
        tids = started_by_msg.get(mid, [])
        if len(tids) > 1:
            rep.fail("message_answered_twice", sid, f"message {mid} started turns {tids}")
            continue
        if len(tids) == 1:
            tid = tids[0]
            terms = terminals.get(tid, [])
            if not terms:
                continue  # reported as turn_not_terminal
            kind = min(terms, key=lambda e: e.get("sequence") or 0)["event_type"]
            ok = {"turn.completed"} if m.expect == "completed" else {"turn.completed", "turn.cancelled"}
            if kind not in ok:
                err = (min(terms, key=lambda e: e.get("sequence") or 0).get("data") or {}).get("error")
                rep.fail("message_not_completed", sid, f"message {mid} turn {tid} ended {kind} {str(err)[:120]}")
            else:
                rep.counts["messages_answered"] += 1
                if m.done_at and m.sent_at:
                    timings[f"e2e_ms:{run.kind}"].append((m.done_at - m.sent_at) * 1000)
                inp = same[0] if same else None
                if inp is not None:
                    timings["pickup_ms"].append(
                        (parse_ts(started[tid]["ts"]) - parse_ts(inp["ts"])) * 1000
                    )
            continue
        # No turn of its own: it must have been steered into a running turn,
        # and that turn must have read it (a reason step after it).
        if not same:
            rep.fail("message_lost", sid, f"message {mid} (delivery={m.delivery}) has no input.message event")
            continue
        seq = same[0].get("sequence") or 0
        host = None
        for tid, s in started.items():
            if (s.get("sequence") or 0) < seq:
                terms = terminals.get(tid, [])
                end = min((t.get("sequence") or 0 for t in terms), default=10**12)
                if end > seq:
                    host = tid
        if host is None:
            rep.fail(
                "message_unanswered",
                sid,
                f"message {mid} delivery={m.delivery} seq {seq}: no turn started for it and no turn was open",
            )
            continue
        read = [
            e
            # A reason step re-run after a crash (`reason.recovered`, mode
            # restart) rebuilds its prompt from the log, so it reads the
            # steered message too (verified in the llm.generation input).
            for e in by_type["reason.started"] + by_type["reason.recovered"]
            if turn_id_of(e) == host and (e.get("sequence") or 0) > seq
        ]
        if not read:
            rep.fail(
                "steered_message_unread",
                sid,
                f"message {mid} delivery={m.delivery} steered into turn {host} at seq {seq}, "
                f"but that turn never reasoned again before ending",
            )
        else:
            rep.counts["messages_answered"] += 1
            rep.counts["messages_steered"] += 1

    # Nothing left running.
    status = (session or {}).get("status")
    if status not in ("idle",):
        rep.fail("session_not_idle", sid, f"status={status}")


OVERLAP_TOLERANCE_S = 0.1


def worker_overlap(run_dir: str, n_workers: int, db_url: str, kills: list, rep: Report):
    """Sessions whose turn steps ran on two workers at the same time.

    Main's worker logs one line per claimed task when it starts executing
    (`Executing task`, with task_id and workflow_id) and a closing line naming
    the same task_id (`Task completed successfully`, `Task completion
    rejected`, or a failure). A task's span on a worker runs from the first to
    the last line naming its task_id; a span still open when the worker was
    SIGKILLed ends at the kill. The workflow -> session mapping is the one
    thing the public API does not expose, so it is read (read-only) from
    `durable_workflow_instances.input->>'session_id'`. Two spans of one
    session on different workers that intersect are a violation; a
    `Task completion rejected` line is reported separately, since it means the
    worker ran a step it had already lost.
    """
    ts_re = re.compile(r"(\d{4}-\d{2}-\d{2}T[\d:.]+Z)")
    task_re = re.compile(r"task_id[=:]\s*\"?([0-9a-f-]{36})")
    wf_re = re.compile(r"workflow_id[=:]\s*\"?(?:Some\()?([0-9a-f-]{36})")
    spans = {}  # (worker, task) -> [start, end, workflow]
    rejected = []
    for j in range(n_workers):
        name = f"worker-{j}"
        path = os.path.join(run_dir, f"{name}.log")
        if not os.path.exists(path):
            continue
        open_tasks = set()
        with open(path, errors="replace") as f:
            for line in f:
                line = re.sub(r"\x1b\[[0-9;]*m", "", line)
                if line.startswith("===== chaos: SIGKILL"):
                    t = float(line.split(" at ")[1].split()[0])
                    for k in open_tasks:
                        spans[k][1] = t
                    open_tasks = set()
                    continue
                tm = task_re.search(line)
                if not tm:
                    continue
                m = ts_re.search(line)
                if not m:
                    continue
                t = parse_ts(m.group(1))
                key = (name, tm.group(1))
                if "Executing task" in line:
                    wf = wf_re.search(line)
                    spans[key] = [t, t, wf.group(1) if wf else None]
                    open_tasks.add(key)
                elif key in spans:
                    spans[key][1] = t
                    if "Task completed successfully" in line or "rejected" in line or "failed" in line.lower():
                        open_tasks.discard(key)
                if "Task completion rejected" in line:
                    rejected.append((name, tm.group(1)))
    wfs = {v[2] for v in spans.values() if v[2]}
    wf_session = {}
    if wfs:
        out = subprocess.run(
            ["psql", db_url, "-At", "-F", "\t", "-c",
             "select id, input->>'session_id' from durable_workflow_instances"],
            capture_output=True, text=True, timeout=120,
        )
        for row in out.stdout.splitlines():
            parts = row.split("\t")
            if len(parts) == 2:
                wf_session[parts[0]] = parts[1]
    by_session = defaultdict(list)
    unmapped = 0
    for (worker, task), (a, b, wf) in spans.items():
        s = wf_session.get(wf or "")
        if not s:
            unmapped += 1
            continue
        by_session[s].append((a, b, worker, task))
    rep.counts["worker_task_spans"] = len(spans)
    rep.counts["worker_task_spans_unmapped"] = unmapped
    for s, sp in by_session.items():
        sp.sort()
        for i in range(len(sp)):
            for k in range(i + 1, len(sp)):
                a, b = sp[i], sp[k]
                if b[0] >= a[1]:
                    break
                # Tolerance: a step's closing log line is written just after
                # the store commit that lets another worker claim the next step.
                if a[2] != b[2] and min(a[1], b[1]) - b[0] > OVERLAP_TOLERANCE_S:
                    rep.fail(
                        "session_on_two_workers",
                        s,
                        f"{a[2]} task {a[3][:8]} {a[0]:.3f}-{a[1]:.3f} overlaps "
                        f"{b[2]} task {b[3][:8]} {b[0]:.3f}-{b[1]:.3f}",
                    )
    for worker, task in rejected:
        rep.fail("task_completion_rejected", worker, f"task {task} ran on a worker that had lost its claim")
    return len(spans)


def summarize(values):
    if not values:
        return "n/a"
    return f"n={len(values)} p50={pct(values,50):.0f} p95={pct(values,95):.0f} p99={pct(values,99):.0f} max={max(values):.0f}"


def mean(v):
    return statistics.fmean(v) if v else None


class ActorLeaseSampler:
    """Samples `session_actors` (read-only) during the run, for builds where
    a worker runs a session's whole turn on a lease (actor cutover, step 6c).

    The row holds one (holder, fence) at a time, so two workers can only run
    one session at once if a holder keeps working after its lease moved on.
    The sampler records every (holder, fence) a session showed, with first and
    last time seen, and flags:
    - a fence that went backwards, or one fence seen with two holders;
    - a takeover: a new holder seen while the previous holder's lease was
      still unexpired at its last sample, with no release in between (the
      row showed no holder). A takeover after expiry or release is normal.
    """

    SQL = (
        "select session_id, coalesce(lease_holder,''), lease_fence, "
        "coalesce(extract(epoch from lease_expires_at)::text,''), extract(epoch from now()) "
        "from session_actors where updated_at > now() - interval '10 minutes'"
    )

    def __init__(self, db_url: str, interval: float = 0.25):
        self.db_url = db_url
        self.interval = interval
        self.samples = 0
        self.history = defaultdict(list)  # session -> [(t, holder, fence, expires)]
        self.stop = False

    @staticmethod
    def available(db_url: str) -> bool:
        out = subprocess.run(
            ["psql", db_url, "-At", "-c", "select to_regclass('public.session_actors') is not null"],
            capture_output=True, text=True, timeout=30,
        )
        return out.stdout.strip() == "t"

    def run(self):
        import time as _t

        while not self.stop:
            out = subprocess.run(
                ["psql", self.db_url, "-At", "-F", "\t", "-c", self.SQL],
                capture_output=True, text=True, timeout=30,
            )
            if out.returncode == 0:
                self.samples += 1
                for row in out.stdout.splitlines():
                    p = row.split("\t")
                    if len(p) != 5:
                        continue
                    sid, holder, fence, exp, now = p
                    h = self.history[sid]
                    rec = (float(now), holder, int(fence), float(exp) if exp else None)
                    if not h or h[-1][1:3] != rec[1:3]:
                        h.append(rec)
                    else:
                        h[-1] = (h[-1][0], holder, int(fence), rec[3])  # keep first-seen time
            _t.sleep(self.interval)

    def check(self, rep: Report, releases: dict | None = None):
        """`releases`: session -> [(t, worker)] from the workers' "Actor
        released" debug lines; a takeover right after the previous holder
        released (between two samples) is not a takeover before expiry."""
        releases = releases or {}
        holders = set()
        for sid, h in self.history.items():
            fence_holder = {}
            last_fence = -1
            prev = None
            for t, holder, fence, exp in h:
                if fence < last_fence:
                    rep.fail("actor_fence_regressed", sid, f"fence {last_fence} -> {fence} at {t:.3f}")
                last_fence = max(last_fence, fence)
                if holder:
                    holders.add(holder)
                    if fence in fence_holder and fence_holder[fence] != holder:
                        rep.fail("actor_fence_two_holders", sid, f"fence {fence}: {fence_holder[fence]} and {holder}")
                    fence_holder[fence] = holder
                    released = any(
                        prev and w in prev[1] and prev[0] - 1.0 <= rt <= t + 0.5
                        for rt, w in releases.get(sid.replace("session_", "").replace("-", ""), [])
                    )
                    if prev and prev[1] and prev[1] != holder and prev[3] and prev[3] > t and not released:
                        rep.fail(
                            "actor_takeover_before_expiry",
                            sid,
                            f"{prev[1]} (fence {prev[2]}, expires {prev[3]:.3f}) -> {holder} (fence {fence}) at {t:.3f}",
                        )
                prev = (t, holder, fence, exp)
        rep.counts["actor_samples"] = self.samples
        rep.counts["actor_sessions_seen"] = len(self.history)
        rep.counts["actor_holders_seen"] = len(holders)
        rep.counts["actor_lease_changes"] = sum(
            len({(x[1], x[2]) for x in h if x[1]}) for h in self.history.values()
        )


def actor_releases(run_dir: str, n_workers: int) -> dict:
    """session uuid-hex -> [(t, worker)] from "Actor released" debug lines."""
    ts_re = re.compile(r"(\d{4}-\d{2}-\d{2}T[\d:.]+Z)")
    sid_re = re.compile(r"session_id[=:]\s*\"?(?:session_)?([0-9a-f-]{32,36})")
    out = defaultdict(list)
    for j in range(n_workers):
        path = os.path.join(run_dir, f"worker-{j}.log")
        if not os.path.exists(path):
            continue
        with open(path, errors="replace") as f:
            for line in f:
                if "Actor released" not in line:
                    continue
                line = re.sub(r"\x1b\[[0-9;]*m", "", line)
                m, s = ts_re.search(line), sid_re.search(line)
                if m and s:
                    out[s.group(1).replace("-", "")].append((parse_ts(m.group(1)), f"chaos-worker-{j}"))
    return out


SPAN_END = ("Actor released", "Actor lease lost", "the lease will expire")


def actor_worker_overlap(run_dir: str, n_workers: int, rep: Report) -> int:
    """Actor builds: sessions whose turn work was logged by two workers at once.

    Events carry no worker id, so this reads the workers' own logs: every
    line naming `session_id=session_<hex>` is activity of that worker on that
    session. A span opens at the first such line, extends with each later
    one, and closes at that worker's `Actor released`, lease-lost or
    "the lease will expire" line for the session, or at a SIGKILL marker (a killed worker does no more work). Spans of one
    session on two different workers that intersect by more than the
    tolerance mean the session ran on two workers at the same time; this is
    the log-side complement of the `session_actors` lease sampler.
    """
    ts_re = re.compile(r"(\d{4}-\d{2}-\d{2}T[\d:.]+Z)")
    sid_re = re.compile(r"session_id[=:]\s*\"?session_([0-9a-f]{32})")
    spans = []  # (start, end, worker, session)
    for j in range(n_workers):
        name = f"chaos-worker-{j}"
        path = os.path.join(run_dir, f"worker-{j}.log")
        if not os.path.exists(path):
            continue
        open_spans = {}  # session -> [start, end]
        with open(path, errors="replace") as f:
            for line in f:
                line = re.sub(r"\x1b\[[0-9;]*m", "", line)
                if line.startswith("===== chaos: SIGKILL"):
                    t = float(line.split(" at ")[1].split()[0])
                    for s, (a, b) in open_spans.items():
                        spans.append((a, b, name, s))
                    open_spans = {}
                    continue
                sm = sid_re.search(line)
                if not sm:
                    continue
                m = ts_re.search(line)
                if not m:
                    continue
                t = parse_ts(m.group(1))
                s = sm.group(1)
                sp = open_spans.setdefault(s, [t, t])
                sp[1] = t
                # The worker is done with the lease: released, lost, or left
                # to expire after a failure (another worker resumes the turn
                # only after expiry, so activity stops here).
                if any(k in line for k in SPAN_END):
                    spans.append((sp[0], sp[1], name, s))
                    del open_spans[s]
        for s, (a, b) in open_spans.items():
            spans.append((a, b, name, s))
    by_session = defaultdict(list)
    for sp in spans:
        by_session[sp[3]].append(sp)
    rep.counts["actor_worker_spans"] = len(spans)
    for s, sp in by_session.items():
        sp.sort()
        for i in range(len(sp)):
            for k in range(i + 1, len(sp)):
                a, b = sp[i], sp[k]
                if b[0] >= a[1]:
                    break
                if a[2] != b[2] and min(a[1], b[1]) - b[0] > OVERLAP_TOLERANCE_S:
                    rep.fail(
                        "session_on_two_workers",
                        "session_" + s,
                        f"{a[2]} {a[0]:.3f}-{a[1]:.3f} overlaps {b[2]} {b[0]:.3f}-{b[1]:.3f}",
                    )
    return len(spans)
