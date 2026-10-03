#!/usr/bin/env python3
"""Keep database drivers and connection constructors at their owning boundary."""

import json
from pathlib import Path
import re
import subprocess
import sys

OWNERS = {"everruns-server", "everruns-durable"}
DRIVERS = {
    "sqlx", "rusqlite", "diesel", "postgres", "tokio-postgres", "mysql",
    "mysql_async", "mongodb", "libsql", "duckdb", "sea-orm", "surrealdb",
}
CONSTRUCTOR = re.compile(
    r"\b(?:PgPool(?:Options)?|PostgresPool|PgConnection|SqlitePool(?:Options)?|"
    r"SqliteConnection|MySqlPool(?:Options)?|Connection|QueryConnection)\s*::\s*"
    r"(?:connect(?:_lazy)?|open(?:_in_memory)?|new)\s*\("
)
RAW_SQLX_CONNECTION = re.compile(
    r"\b(?:PgPool(?:Options)?|PgConnection|PgConnectOptions|"
    r"SqlitePool(?:Options)?|SqliteConnection|SqliteConnectOptions|"
    r"MySqlPool(?:Options)?|MySqlConnection|MySqlConnectOptions)\b|"
    r"\bsqlx\s*::\s*(?:Pool|pool\s*::\s*PoolOptions|Connection|ConnectOptions)\b|"
    r"\bsqlx\s*::\s*\{[^}]*\b(?:Pool|PoolOptions|Connection|ConnectOptions)\b",
    re.DOTALL,
)


def dependency_violations(packages):
    violations = []
    for package in packages:
        if package["name"] in OWNERS:
            continue
        for dependency in package["dependencies"]:
            if dependency["name"] not in DRIVERS or dependency["kind"] == "dev":
                continue
            # TypedId owns its optional PostgreSQL codecs (the orphan rule).
            # This exception grants no connection ownership; source is checked below.
            if (
                package["name"] == "everruns-contracts"
                and dependency["name"] == "sqlx"
                and dependency["kind"] is None
                and dependency["optional"]
                and dependency.get("rename") in (None, "sqlx")
            ):
                continue
            violations.append(
                f'{package["name"]}: {dependency["kind"] or "normal"} database driver '
                f'{dependency["name"]} (alias={dependency.get("rename") or dependency["name"]})'
            )
    return violations


def constructor_violations(packages):
    violations = []
    for package in packages:
        if package["name"] in OWNERS:
            continue
        root = Path(package["manifest_path"]).parent / "src"
        for source in root.rglob("*.rs"):
            text = "\n".join(
                "" if line.lstrip().startswith("//") else line
                for line in source.read_text().splitlines()
            )
            for match in CONSTRUCTOR.finditer(text):
                line_number = text.count("\n", 0, match.start()) + 1
                violations.append(f"{source}:{line_number}: raw database connection constructor")
            # The optional codec edge must not let contracts import a pool
            # under an alias and then hide its constructor behind that name.
            if package["name"] == "everruns-contracts":
                for match in RAW_SQLX_CONNECTION.finditer(text):
                    line_number = text.count("\n", 0, match.start()) + 1
                    violations.append(f"{source}:{line_number}: database connection type outside owner")
    return violations


def main():
    result = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        text=True, capture_output=True,
    )
    if result.returncode:
        print("Database guard could not run: cargo metadata failed.", file=sys.stderr)
        print(result.stderr, file=sys.stderr)
        return 2
    metadata = json.loads(result.stdout)
    members = set(metadata["workspace_members"])
    packages = [package for package in metadata["packages"] if package["id"] in members]
    violations = dependency_violations(packages) + constructor_violations(packages)
    if violations:
        print("\n".join(violations), file=sys.stderr)
        print("Database drivers and connection construction belong to server or durable.", file=sys.stderr)
        return 1
    print("Database driver isolation guard passed: only server/durable own connections; contracts codecs remain optional.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
