#!/usr/bin/env python3
"""Keep database drivers and connection constructors at their owning boundary."""

import json
from pathlib import Path
import re
import subprocess
import sys

# everruns-db owns embedded SQLite construction for the facade and serve hosts.
# everruns-pg-embedded runs the server's throwaway dev/test cluster: it creates
# and drops databases and reads no tables.
OWNERS = {"everruns-server", "everruns-durable", "everruns-db", "everruns-pg-embedded"}
DRIVERS = {
    "sqlx", "rusqlite", "diesel", "postgres", "tokio-postgres", "mysql",
    "mysql_async", "mongodb", "libsql", "duckdb", "sea-orm", "surrealdb",
}
CONNECTION_TYPES = {
    "PgPool", "PgPoolOptions", "PostgresPool", "PgConnection",
    "SqlitePool", "SqlitePoolOptions", "SqliteConnection",
    "MySqlPool", "MySqlPoolOptions", "Connection", "QueryConnection",
}


def connection_constructor_pattern(text):
    # Reexported query handles retain their driver type identity. Follow local
    # import/type aliases so a callback type cannot hide an owning constructor.
    names = set(CONNECTION_TYPES)
    imports = re.findall(r"\b([A-Za-z_]\w*)\s+as\s+([A-Za-z_]\w*)", text)
    aliases = re.findall(r"\btype\s+([A-Za-z_]\w*)(?:\s*<[^=;]*>)?\s*=\s*([^;]+);", text)
    while True:
        before = len(names)
        for original, alias in imports:
            if original in names:
                names.add(alias)
        for alias, target in aliases:
            base = target.split("<", 1)[0].strip().split("::")[-1].strip()
            if base in names:
                names.add(alias)
        if len(names) == before:
            break
    return re.compile(
        r"\b(?:" + "|".join(re.escape(name) for name in sorted(names)) + r")"
        r"(?:\s*::\s*<[^;()]*>)?\s*::\s*"
        r"(?:connect(?:_[A-Za-z0-9_]+)?|open(?:_[A-Za-z0-9_]+)?|new)\s*\("
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
            for match in connection_constructor_pattern(text).finditer(text):
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
        print("Database drivers and connection construction belong to server, durable or everruns-db.", file=sys.stderr)
        return 1
    print("Database driver isolation guard passed: only server/durable/db own connections; contracts codecs remain optional.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
