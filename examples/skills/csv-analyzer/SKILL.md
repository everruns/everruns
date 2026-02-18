---
name: csv-analyzer
description: Analyze CSV files to generate summary statistics, detect data quality issues, and produce formatted reports.
license: MIT
compatibility: Python 3.10+
metadata:
  category: data-processing
  version: "1.0"
allowed-tools: read_file write_file virtual_bash
---

# CSV Analyzer

Analyze CSV data files to produce summary statistics and quality reports.

## When to Use

Activate this skill when a user provides a CSV file and wants:
- Summary statistics (row count, column types, missing values)
- Data quality analysis (duplicates, outliers, formatting issues)
- A formatted report

## Instructions

1. Read the CSV file using the `read_file` tool
2. Run the analysis script at `/skills/csv-analyzer/scripts/analyze.py`
3. Review the output and present findings to the user

## Analysis Steps

### Step 1: Load Data
```bash
python3 /skills/csv-analyzer/scripts/analyze.py /workspace/data.csv
```

### Step 2: Review Output
The script outputs a JSON report with:
- `row_count`: Total number of rows
- `column_count`: Number of columns
- `columns`: Array of column metadata (name, type, null_count, unique_count)
- `quality_issues`: Array of detected problems

### Step 3: Present Results
Format the results as a readable summary table for the user.

## References

See `/skills/csv-analyzer/references/` for additional documentation on CSV analysis best practices.
