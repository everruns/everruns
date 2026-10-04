#!/usr/bin/env python3
"""Check the hand-written docs catalogs against the code they describe.

The capability index, the built-in harness pages, the event reference and the
environment-variable summary are hand-written tables that drifted from the code.
This check compares each against its source of truth without compiling Rust:

- Capabilities: ``docs/api/capability-catalog.json``, the registry snapshot that
  ``crates/integrations-catalog/src/docs_catalog.rs`` writes and keeps fresh.
- Generic and Platform Chat Agent: the capability lists in ``crates/contracts/src/capability/presets.rs``
  (generic) and ``crates/server/src/platform_chat_agent.rs``.
- Events: the event-type constants in ``crates/core/src/events/mod.rs``.
- Environment variables: string literals the Rust sources read.

Run from anywhere; ``--root`` points at a checkout (tests use a scratch copy).
"""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys

CATALOG = "docs/api/capability-catalog.json"
CAPABILITY_INDEX = "docs/capabilities/index.md"
CAPABILITY_PAGES = "docs/capabilities"
EVENT_REFERENCE = "docs/event-reference.md"
EVENT_SOURCE = "crates/core/src/events/mod.rs"
ENV_PAGE = "docs/sre/environment-variables.md"
GENERIC_PAGE = "docs/built-ins/harnesses/generic.md"
GENERIC_SOURCE = "crates/contracts/src/capability/presets.rs"
PLATFORM_CHAT_PAGE = "docs/built-ins/harnesses/platform-chat.md"
PLATFORM_CHAT_SOURCE = "crates/server/src/platform_chat_agent.rs"
SERVER_MANIFEST = "crates/server/Cargo.toml"
CATALOG_MANIFEST = "crates/integrations-catalog/Cargo.toml"

# Production-grade capabilities the index deliberately leaves out. Each one is
# a building block a harness composes rather than something a user picks from
# the catalog; the harness pages list them. Adding a capability here is a
# decision, so give the reason.
INDEX_EXEMPT = {
    "btw": "harness-composed side-question command",
    "error_disclosure": "harness-composed error detail setting",
    "human_intent": "harness-composed tool-call narration",
    "tool_output_distillation": "harness-composed output handling",
    "tool_output_persistence": "harness-composed output handling",
}

TABLE_ROW = re.compile(r"^\|(.+)\|\s*$")
LINK = re.compile(r"\[([^\]]+)\]\(([^)]+)\)")
CAPABILITY_LINK = re.compile(r"^/capabilities/([a-z0-9-]+)/?(?:#.*)?$")
CODE_SPAN = re.compile(r"`([^`]+)`")
LEADING_INT = re.compile(r"^\s*(\d+)\b")


class Report:
    def __init__(self) -> None:
        self.errors: list[str] = []

    def error(self, where: str, message: str) -> None:
        self.errors.append(f"{where}: {message}")


def read(root: pathlib.Path, relative: str) -> str:
    return (root / relative).read_text(encoding="utf-8")


def table_rows(lines: list[str]) -> list[list[str]]:
    """Body rows of every Markdown table in ``lines``, header and rule skipped."""
    rows: list[list[str]] = []
    in_table = False
    for line in lines:
        match = TABLE_ROW.match(line.strip())
        if not match:
            in_table = False
            continue
        cells = [cell.strip() for cell in match.group(1).split("|")]
        if not in_table:
            in_table = True  # header row
            continue
        if all(re.fullmatch(r":?-+:?", cell) for cell in cells if cell):
            continue
        rows.append(cells)
    return rows


def section(text: str, heading: str) -> list[str]:
    """Lines under a ``## heading`` up to the next ``## `` heading."""
    lines = text.splitlines()
    out: list[str] = []
    inside = False
    for line in lines:
        if line.startswith("## "):
            if inside:
                break
            inside = line[3:].strip() == heading
            continue
        if inside:
            out.append(line)
    return out


def subsection(lines: list[str], heading: str) -> list[str]:
    out: list[str] = []
    inside = False
    for line in lines:
        if line.startswith("### "):
            if inside:
                break
            inside = line[4:].strip() == heading
            continue
        if inside:
            out.append(line)
    return out


# --- capabilities -----------------------------------------------------------


def load_catalog(root: pathlib.Path) -> dict[str, dict]:
    data = json.loads(read(root, CATALOG))
    return {entry["id"]: entry for entry in data["capabilities"]}


def listed(entry: dict) -> bool:
    return entry["status"] != "retired"


def must_be_indexed(entry: dict) -> bool:
    """A capability every production deployment offers by default."""
    return (
        entry["status"] in ("available", "deprecated")
        and "prod" in entry["grades"]
        and entry.get("feature_flag") is None
    )


class Resolver:
    """Maps a table cell (a link or a plain label) to a capability id.

    The index is the reference: its rows bind a link target and a label to an
    id, and other tables (dependencies, harness pages) resolve against that.
    Registry display names are the fallback for labels.
    """

    def __init__(self, catalog: dict[str, dict]) -> None:
        self.by_href: dict[str, str] = {}
        self.by_label = {entry["name"].lower(): capability_id for capability_id, entry in catalog.items()}

    @staticmethod
    def _href(href: str) -> str:
        # Keep the anchor: several capabilities can share one page (tool
        # search), and the anchor is what tells their rows apart.
        base, _, anchor = href.partition("#")
        base = base.rstrip("/")
        return f"{base}#{anchor}" if anchor else base

    def bind(self, cell: str, capability_id: str) -> None:
        link = LINK.search(cell)
        label = link.group(1) if link else cell
        if link:
            self.by_href[self._href(link.group(2))] = capability_id
        self.by_label[label.strip().lower()] = capability_id

    def resolve(self, cell: str) -> str | None:
        link = LINK.search(cell)
        if link:
            found = self.by_href.get(self._href(link.group(2)))
            if found:
                return found
            cell = link.group(1)
        return self.by_label.get(cell.strip().lower())

    def resolve_all(self, cell: str) -> list[str | None]:
        parts = [part for part in re.split(r",\s*(?![^\[]*\])", cell) if part.strip()]
        return [self.resolve(part) for part in parts]


def check_capability_index(
    root: pathlib.Path, catalog: dict[str, dict], sources: "RustSources", report: Report
) -> Resolver:
    """Returns the cell resolver the index establishes."""
    where = CAPABILITY_INDEX
    text = read(root, where)
    reference = section(text, "Capability Reference")
    resolver = Resolver(catalog)
    seen: set[str] = set()
    for cells in table_rows(reference):
        if len(cells) < 3:
            continue
        row = " ".join(cells)
        ids = CODE_SPAN.findall(cells[1])
        if len(ids) != 1:
            report.error(where, f"row {cells[0]!r} must name exactly one capability id")
            continue
        capability_id = ids[0]
        if capability_id in seen:
            report.error(where, f"`{capability_id}` is listed twice")
        seen.add(capability_id)
        resolver.bind(cells[0], capability_id)
        link = LINK.search(cells[0])
        slug_match = CAPABILITY_LINK.match(link.group(2)) if link else None
        entry = catalog.get(capability_id)
        if entry is None:
            # Framework-only capabilities (host_shell) are not in the hosted
            # registry; the row has to say so, and the id has to exist in code.
            if not re.search(r"framework[- ]only", row, re.IGNORECASE):
                report.error(where, f"`{capability_id}` is not a registered capability")
            elif not sources.has_literal(capability_id):
                report.error(where, f"Framework-only `{capability_id}` is not defined in any Rust source")
            continue
        if not listed(entry):
            report.error(where, f"`{capability_id}` is {entry['status']} and hidden from catalogs")
        if slug_match:
            slug = slug_match.group(1)
            docs_slug = entry.get("docs_slug")
            if docs_slug != slug:
                report.error(
                    where,
                    f"`{capability_id}` links /capabilities/{slug}/ but the UI docs link "
                    f"(builtin_capability_docs_slug in crates/core/src/capability_dto.rs) is "
                    f"{docs_slug!r}",
                )
        count = LEADING_INT.match(cells[2])
        tools = len(entry["tools"])
        if count is None:
            report.error(where, f"`{capability_id}` tool count {cells[2]!r} is not a number")
        elif int(count.group(1)) != tools:
            report.error(
                where,
                f"`{capability_id}` lists {count.group(1)} tools, the registry has {tools} "
                f"({', '.join(entry['tools']) or 'none'})",
            )
        if "prod" not in entry["grades"] and "dev-only" not in row.lower():
            report.error(where, f"`{capability_id}` is registered only at dev grade; mark the row dev-only")
        flag = entry.get("feature_flag")
        if flag and f"FEATURE_{flag.upper()}" not in row:
            report.error(where, f"`{capability_id}` needs `FEATURE_{flag.upper()}`; say so in the row")

    for capability_id, entry in sorted(catalog.items()):
        if not must_be_indexed(entry) or capability_id in seen or capability_id in INDEX_EXEMPT:
            continue
        report.error(
            where,
            f"production capability `{capability_id}` ({entry['name']}) is missing from the index "
            f"(add a row, or exempt it in INDEX_EXEMPT in scripts/check_docs_catalogs.py)",
        )
    for capability_id in sorted(INDEX_EXEMPT):
        if capability_id not in catalog:
            report.error("scripts/check_docs_catalogs.py", f"INDEX_EXEMPT names unknown capability `{capability_id}`")
        elif capability_id in seen:
            report.error("scripts/check_docs_catalogs.py", f"INDEX_EXEMPT names indexed capability `{capability_id}`")

    pages = root / CAPABILITY_PAGES
    for capability_id, entry in sorted(catalog.items()):
        docs_slug = entry.get("docs_slug")
        if docs_slug and not any((pages / f"{docs_slug}{ext}").exists() for ext in (".md", ".mdx")):
            report.error(
                "crates/core/src/capability_dto.rs",
                f"`{capability_id}` docs slug {docs_slug!r} has no page under {CAPABILITY_PAGES}/",
            )

    check_dependencies_table(text, catalog, resolver, seen, report)
    return resolver


def check_dependencies_table(
    text: str, catalog: dict[str, dict], resolver: Resolver, indexed: set[str], report: Report
) -> None:
    where = f"{CAPABILITY_INDEX} (Dependencies)"
    rows = table_rows(subsection(section(text, "Key Concepts"), "Dependencies"))
    documented: dict[str, set[str]] = {}
    for cells in rows:
        if len(cells) < 2:
            continue
        capability_id = resolver.resolve(cells[0])
        deps = resolver.resolve_all(cells[1])
        if capability_id is None or None in deps:
            report.error(where, f"row {' | '.join(cells)!r} names a capability the index does not list")
            continue
        documented[capability_id] = set(deps)  # type: ignore[arg-type]
    for capability_id in sorted(indexed):
        entry = catalog.get(capability_id)
        if entry is None:
            continue
        actual = set(entry["dependencies"])
        shown = documented.get(capability_id, set())
        if actual != shown:
            report.error(
                where,
                f"`{capability_id}` depends on {sorted(actual) or 'nothing'}, the table says {sorted(shown) or 'nothing'}",
            )


# --- harnesses --------------------------------------------------------------

CAPABILITY_CALL = re.compile(
    r'(?:CapabilityRef|BuiltInCapabilityDefinition)::(?:new|with_config)\(\s*"([a-z0-9_]+)"'
)


def harness_source_ids(text: str, start: str, end: str) -> list[str]:
    begin = text.find(start)
    if begin < 0:
        return []
    finish = text.find(end, begin)
    return CAPABILITY_CALL.findall(text[begin : finish if finish >= 0 else len(text)])


def agent_source_ids(text: str) -> list[str]:
    begin = text.find("let capabilities = vec![")
    if begin < 0:
        return []
    finish = text.find("];", begin)
    return re.findall(r'\("([a-z_]+)"\.into\(\),', text[begin:finish])


def check_harness(
    root: pathlib.Path,
    page: str,
    source: str,
    ids: list[str],
    catalog: dict[str, dict],
    resolver: Resolver,
    report: Report,
) -> None:
    if not ids:
        report.error(source, "could not find the harness capability list (did the code move?)")
        return
    text = read(root, page)
    documented: list[str] = []
    for cells in table_rows(section(text, "Bundled Capabilities")):
        capability_id = resolver.resolve(cells[0])
        if capability_id is None:
            report.error(page, f"row {cells[0]!r} matches no capability (link a capability page or use the registry name)")
            continue
        documented.append(capability_id)
    for capability_id in sorted(set(ids) - set(documented)):
        name = catalog.get(capability_id, {}).get("name", capability_id)
        report.error(page, f"missing `{capability_id}` ({name}), which {source} configures")
    for capability_id in sorted(set(documented) - set(ids)):
        report.error(page, f"lists `{capability_id}`, which {source} does not configure")
    for capability_id in sorted({c for c in documented if documented.count(c) > 1}):
        report.error(page, f"lists `{capability_id}` twice")
    for count in re.findall(r"configures (\d+) capabilities", text):
        if int(count) != len(ids):
            report.error(page, f"says it configures {count} capabilities, {source} configures {len(ids)}")


# --- events -----------------------------------------------------------------

EVENT_CONST = re.compile(r'^pub const [A-Z0-9_]+: &str = "([a-z_]+(?:\.[a-z_]+)+)";', re.MULTILINE)


def check_events(root: pathlib.Path, report: Report) -> None:
    source = read(root, EVENT_SOURCE)
    defined = set(EVENT_CONST.findall(source))
    if len(defined) < 10:
        report.error(EVENT_SOURCE, "found too few event constants (did the format change?)")
        return
    text = read(root, EVENT_REFERENCE)
    table: list[str] = []
    for cells in table_rows(section(text, "All event types")):
        names = CODE_SPAN.findall(cells[0])
        if len(names) != 1:
            report.error(EVENT_REFERENCE, f"row {cells[0]!r} must name one event type")
            continue
        table.append(names[0])
    for name in sorted(defined - set(table)):
        report.error(EVENT_REFERENCE, f"`{name}` is defined in {EVENT_SOURCE} but missing from the table")
    for name in sorted(set(table) - defined):
        report.error(EVENT_REFERENCE, f"`{name}` is in the table but not defined in {EVENT_SOURCE}")
    for name in sorted({n for n in table if table.count(n) > 1}):
        report.error(EVENT_REFERENCE, f"`{name}` is listed twice")
    for line in text.splitlines():
        if line.startswith("### ") and "." in line:
            name = line[4:].strip()
            if name not in defined and "retired" not in name.lower():
                report.error(EVENT_REFERENCE, f"section `{name}` documents an event type that is not defined")
    for count in re.findall(r"\b(\d+) event types\b", text):
        if int(count) != len(defined):
            report.error(EVENT_REFERENCE, f"says {count} event types, {EVENT_SOURCE} defines {len(defined)}")


# --- environment variables --------------------------------------------------

ENV_NAME = re.compile(r"^[A-Z][A-Z0-9_]*(?:\*)?$")


class RustSources:
    """Concatenated Rust sources under crates/ and integrations/, read once."""

    def __init__(self, root: pathlib.Path) -> None:
        self.root = root
        self._text: str | None = None

    @property
    def text(self) -> str:
        if self._text is None:
            chunks: list[str] = []
            for tree in ("crates", "integrations"):
                for path in sorted((self.root / tree).rglob("*.rs")):
                    if "target" in path.parts:
                        continue
                    chunks.append(path.read_text(encoding="utf-8", errors="replace"))
            self._text = "\n".join(chunks)
        return self._text

    def has_literal(self, value: str) -> bool:
        return f'"{value}"' in self.text

    def has_literal_prefix(self, prefix: str) -> bool:
        return re.search(r'"' + re.escape(prefix) + r'[A-Z0-9_]*"', self.text) is not None


def check_env_summary(root: pathlib.Path, sources: RustSources, report: Report) -> None:
    text = read(root, ENV_PAGE)
    rows = table_rows(section(text, "Summary"))
    if not rows:
        report.error(ENV_PAGE, "missing the `## Summary` table")
        return
    for cells in rows:
        for name in CODE_SPAN.findall(cells[0]):
            if not ENV_NAME.match(name):
                continue
            if name.endswith("*"):
                found = sources.has_literal_prefix(name[:-1])
            else:
                found = sources.has_literal(name)
            if not found:
                report.error(ENV_PAGE, f"summary lists `{name}`, which no Rust source under crates/ or integrations/ reads")


# --- feature parity ---------------------------------------------------------

PLATFORM_FEATURES = re.compile(r'^everruns-platform\s*=\s*\{[^}]*features\s*=\s*\[([^\]]*)\]', re.MULTILINE)


def platform_features(root: pathlib.Path, manifest: str) -> set[str] | None:
    match = PLATFORM_FEATURES.search(read(root, manifest))
    if not match:
        return None
    return set(re.findall(r'"([^"]+)"', match.group(1)))


def check_feature_parity(root: pathlib.Path, report: Report) -> None:
    server = platform_features(root, SERVER_MANIFEST)
    snapshot = platform_features(root, CATALOG_MANIFEST)
    if server is None or snapshot is None:
        report.error(CATALOG_MANIFEST, "could not read the everruns-platform feature lists")
    elif server != snapshot:
        report.error(
            CATALOG_MANIFEST,
            f"the docs catalog snapshot builds everruns-platform with {sorted(snapshot)}, "
            f"the server with {sorted(server)}; keep them equal",
        )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=pathlib.Path, default=pathlib.Path(__file__).resolve().parent.parent)
    args = parser.parse_args()
    root: pathlib.Path = args.root
    report = Report()
    sources = RustSources(root)

    catalog = load_catalog(root)
    resolver = check_capability_index(root, catalog, sources, report)
    check_harness(
        root,
        GENERIC_PAGE,
        GENERIC_SOURCE,
        harness_source_ids(read(root, GENERIC_SOURCE), "pub fn generic_capabilities()", "\n}\n"),
        catalog,
        resolver,
        report,
    )
    check_harness(
        root,
        PLATFORM_CHAT_PAGE,
        PLATFORM_CHAT_SOURCE,
        agent_source_ids(read(root, PLATFORM_CHAT_SOURCE)),
        catalog,
        resolver,
        report,
    )
    check_events(root, report)
    check_env_summary(root, sources, report)
    check_feature_parity(root, report)

    if report.errors:
        print("Docs catalogs drifted from the code:", file=sys.stderr)
        for error in report.errors:
            print(f"  - {error}", file=sys.stderr)
        return 1
    print("Docs catalogs match the code.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
