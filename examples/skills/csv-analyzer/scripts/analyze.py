#!/usr/bin/env python3
"""CSV analysis script for the csv-analyzer skill.

Usage: python3 analyze.py <csv_file_path>

Outputs a JSON report with summary statistics and data quality issues.
"""

import csv
import json
import sys
from collections import Counter


def analyze_csv(filepath: str) -> dict:
    """Analyze a CSV file and return a summary report."""
    with open(filepath, "r", newline="") as f:
        reader = csv.DictReader(f)
        rows = list(reader)

    if not rows:
        return {
            "row_count": 0,
            "column_count": 0,
            "columns": [],
            "quality_issues": ["File is empty or has no data rows"],
        }

    columns = list(rows[0].keys())
    column_stats = []
    quality_issues = []

    for col in columns:
        values = [row.get(col, "") for row in rows]
        null_count = sum(1 for v in values if v is None or v.strip() == "")
        unique_count = len(set(v for v in values if v and v.strip()))

        # Detect column type
        col_type = detect_type(values)

        column_stats.append(
            {
                "name": col,
                "type": col_type,
                "null_count": null_count,
                "unique_count": unique_count,
                "null_percentage": round(null_count / len(values) * 100, 1),
            }
        )

        # Quality checks
        if null_count > len(values) * 0.5:
            quality_issues.append(
                f"Column '{col}' has {null_count}/{len(values)} missing values ({round(null_count/len(values)*100, 1)}%)"
            )

        if unique_count == 1 and len(values) > 1:
            quality_issues.append(
                f"Column '{col}' has only one unique value (constant column)"
            )

    # Check for duplicate rows
    row_strs = ["|".join(row.get(c, "") for c in columns) for row in rows]
    dupes = len(row_strs) - len(set(row_strs))
    if dupes > 0:
        quality_issues.append(f"Found {dupes} duplicate row(s)")

    return {
        "row_count": len(rows),
        "column_count": len(columns),
        "columns": column_stats,
        "quality_issues": quality_issues if quality_issues else ["No issues detected"],
    }


def detect_type(values: list) -> str:
    """Detect the most likely type for a column's values."""
    non_empty = [v for v in values if v and v.strip()]
    if not non_empty:
        return "empty"

    int_count = sum(1 for v in non_empty if is_int(v))
    float_count = sum(1 for v in non_empty if is_float(v))

    if int_count == len(non_empty):
        return "integer"
    if float_count == len(non_empty):
        return "float"
    return "string"


def is_int(s: str) -> bool:
    try:
        int(s.strip())
        return True
    except ValueError:
        return False


def is_float(s: str) -> bool:
    try:
        float(s.strip())
        return True
    except ValueError:
        return False


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print(json.dumps({"error": "Usage: analyze.py <csv_file_path>"}))
        sys.exit(1)

    report = analyze_csv(sys.argv[1])
    print(json.dumps(report, indent=2))
