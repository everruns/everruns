"""Summarize only the measured phase, preserving sampling gaps and failures."""

import json
from pathlib import Path


def summarize(path, start, end, memory_budget, expected_restarts=None):
    expected_restarts = expected_restarts or set()
    samples = []
    for line in Path(path).read_text().splitlines():
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        if start <= row["epoch"] <= end:
            samples.append(row)
    peaks = {}
    cpu_last = {}
    cpu_total = {}
    total_peak = 0
    for row in samples:
        total = 0
        for name, value in row.get("processes", {}).items():
            if "rss_bytes" not in value:
                continue
            peaks[name] = max(peaks.get(name, 0), value["rss_bytes"])
            total += value.get("pss_bytes", value["rss_bytes"])
            # Counters reset when an injected restart changes the role's PID.
            previous = cpu_last.get(name, value["cpu_seconds"])
            cpu_total[name] = cpu_total.get(name, 0) + max(
                0, value["cpu_seconds"] - previous
            )
            cpu_last[name] = value["cpu_seconds"]
        total_peak = max(total_peak, total)
    database = [r["database"] for r in samples if r.get("database")]
    elapsed = samples[-1]["epoch"] - samples[0]["epoch"] if len(samples) > 1 else 0
    checks = {
        "samples_present": len(samples) >= 2,
        "sampling_regular": bool(samples)
        and max(row["sample_gap"] for row in samples) <= 5,
        "samples_cover_phase": bool(samples)
        and samples[0]["epoch"] - start <= 5
        and end - samples[-1]["epoch"] <= 5,
        "sampler_healthy": not any("error" in row for row in samples),
        "memory_within_budget": 0 < total_peak < memory_budget,
        "processes_stayed_alive": not any(
            value.get("missing")
            for row in samples
            for name, value in row.get("processes", {}).items()
            if not name.startswith("generator-")
            and not name.startswith("postgres/")
            and name not in expected_restarts
        ),
    }
    return {
        "passed": all(checks.values()),
        "checks": checks,
        "sample_count": len(samples),
        "max_sample_gap_seconds": max((r["sample_gap"] for r in samples), default=None),
        "peak_process_rss_bytes": peaks,
        "peak_total_resident_bytes": total_peak,
        "memory_budget_bytes": memory_budget,
        "process_cpu_seconds": cpu_total,
        "database": {
            "max_connections": max((r["connections"] for r in database), default=None),
            "max_active": max((r["active"] for r in database), default=None),
            "max_lock_waiters": max(
                (r["lock_waiters"] for r in database), default=None
            ),
            "bytes_growth": database[-1]["database_bytes"]
            - database[0]["database_bytes"]
            if database
            else None,
            "commits_per_second": (database[-1]["commits"] - database[0]["commits"])
            / elapsed
            if database and elapsed
            else None,
        },
    }
