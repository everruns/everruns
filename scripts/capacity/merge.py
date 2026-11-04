"""Merge distributed raw turn samples, never averages of shard percentiles."""

import argparse
import json
from collections import Counter
from pathlib import Path
from .run import percentiles, verdict, queue_metrics


def merge(directories):
    reports = [json.loads((Path(d) / "report.json").read_text()) for d in directories]
    first = reports[0]
    keys = [
        "run_id",
        "path",
        "phase",
        "total_users",
        "shards",
        "interval_seconds",
        "background_polls",
        "environment",
        "fault_mode",
        "requested_duration_seconds",
        "workload",
    ]
    if any(any(r[k] != first[k] for k in keys) for r in reports):
        raise ValueError("shard configurations differ")
    if len(reports) != first["shards"] or {r["shard_index"] for r in reports} != set(
        range(first["shards"])
    ):
        raise ValueError("missing or duplicated shard")
    if (
        max(r["start_epoch"] for r in reports) - min(r["start_epoch"] for r in reports)
        > 1
    ):
        raise ValueError("shards did not start within one second")
    rows = []
    start = min(r["start_epoch"] for r in reports)
    end = max(r["start_epoch"] + r["duration_seconds"] for r in reports)
    horizon = end - start
    for directory, report in zip(directories, reports):
        for line in (Path(directory) / "turns.jsonl").read_text().splitlines():
            row = json.loads(line)
            row["start_seconds"] += report["start_epoch"] - start
            rows.append(row)
    if len({r["marker"] for r in rows}) != len(rows):
        raise ValueError("duplicated attempt across shards")
    result = {k: first[k] for k in keys}
    identities = [identity for r in reports for identity in r["identity_fingerprints"]]
    if len(identities) != len(set(identities)):
        raise ValueError("shards reused an authenticated principal")
    result["identity_fingerprints"] = identities
    for key in [
        "users",
        "connected_users",
        "started_users",
        "scheduled_users",
        "planned_attempts",
        "attempted",
        "admissions",
        "completed",
        "audit_errors",
        "audited_inputs",
        "failed_tools",
        "auth_refreshes",
        "stream_cycles",
        "stream_missing_inputs",
        "terminal_stream_errors",
        "stream_connections",
        "stream_errors",
        "setup_stream_errors",
        "post_phase_stream_errors",
        "replayed_frames",
        "forced_disconnects",
        "poll_requests",
        "workload_audit_errors",
        "workload_verified_turns",
        "reconnected_users",
    ]:
        result[key] = sum(r[key] for r in reports)
    for key in ["errors", "poll_errors", "http_status", "http_versions"]:
        counts = Counter()
        for r in reports:
            counts.update(r[key])
        result[key] = dict(counts)
    for source, metric in [
        ("ack_ms", "ack_ms"),
        ("turn_ms", "turn_ms"),
        ("first_delta_ms", "first_delta_ms"),
        ("pickup_ms", "pickup_ms"),
        ("arrival_lag_ms", "arrival_lag_ms"),
    ]:
        result[metric] = percentiles(
            [r[source] for r in rows if r.get(source) is not None]
        )
    result["generator_start_skew_ms"] = (
        max(r["start_epoch"] for r in reports) - min(r["start_epoch"] for r in reports)
    ) * 1000
    result["start_epoch"] = start
    result["end_epoch"] = end
    result["duration_seconds"] = end - start
    result["completed_turns_per_second"] = (
        result["completed"] / result["duration_seconds"]
    )
    occupancy = []
    queued = []
    for row in rows:
        if row.get("admitted"):
            received = row.get("input_received_ms", row["ack_ms"])
            start = row["start_seconds"] + received / 1000
            end = (
                row["start_seconds"] + row["turn_ms"] / 1000
                if row.get("completed")
                else horizon
            )
            occupancy.extend([(start, 1), (end, -1)])
            claimed = (
                row["start_seconds"] + row["turn_started_ms"] / 1000
                if row.get("turn_started_ms") is not None
                else end
            )
            queued.extend([(start, 1), (claimed, -1)])

    def peak(timeline):
        current = maximum = 0
        for _, change in sorted(timeline):
            current += change
            maximum = max(maximum, current)
        return maximum

    result["client_observed_peak_unfinished_turns"] = peak(occupancy)
    result["client_observed_peak_waiting_turns"] = peak(queued)
    result["queue"] = queue_metrics(rows, first["requested_duration_seconds"])
    result["shard_reports"] = [str(Path(d).name + "/report.json") for d in directories]
    return result


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("directories", nargs="+")
    p.add_argument("--output", required=True)
    p.add_argument("--ack-p95-ms", type=float, default=500)
    a = p.parse_args()
    report = merge(a.directories)
    report.update(verdict(report, a.ack_p95_ms))
    Path(a.output).write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))
    raise SystemExit(0 if report["passed"] else 1)


if __name__ == "__main__":
    main()
