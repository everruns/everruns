#!/usr/bin/env python3
"""Check the Python code blocks in docs/ against the published Python SDK.

Most Python blocks in the docs are fragments: they assume a `client`, an
`agent` or a `session` from an earlier step, and several stream until a turn
ends. They cannot run on their own, but they can be checked against the SDK
they call. For every ```python block this script:

- parses it (top-level `await` / `async for` allowed, as in a notebook);
- checks every `from everruns_sdk import ...` name exists;
- checks every `client.<resource>.<method>(...)` call names a real method and
  passes arguments its signature accepts;
- checks keyword arguments passed to SDK classes (`AgentCapabilityConfig(...)`).

With `--execute`, blocks that are complete programs (they start with an import
and call `asyncio.run(...)`) also run against the server in EVERRUNS_API_URL,
the way CI runs the notebook tutorials against llmsim.

Requires `everruns-sdk` installed. Exits non-zero on any finding.
"""

from __future__ import annotations

import argparse
import ast
import inspect
import os
import re
import subprocess
import sys
import tempfile
import textwrap
from dataclasses import dataclass
from pathlib import Path

FENCE = re.compile(r"^(?P<indent>[ \t]*)```(?:python|py)\b[^\n]*\n(?P<body>.*?)^(?P=indent)```", re.MULTILINE | re.DOTALL)
PROGRAM_TIMEOUT_SECONDS = 300


@dataclass
class Snippet:
    path: Path
    line: int
    source: str

    @property
    def where(self) -> str:
        return f"{self.path}:{self.line}"

    def at(self, node: ast.AST) -> str:
        return f"{self.path}:{self.line + getattr(node, 'lineno', 1) - 1}"

    @property
    def is_program(self) -> bool:
        first = next((line for line in self.source.splitlines() if line.strip()), "")
        return first.startswith(("import ", "from ")) and "asyncio.run(" in self.source


def snippets(docs: Path) -> list[Snippet]:
    found: list[Snippet] = []
    for path in sorted([*docs.rglob("*.md"), *docs.rglob("*.mdx")]):
        text = path.read_text(encoding="utf-8")
        for match in FENCE.finditer(text):
            line = text.count("\n", 0, match.start()) + 2
            found.append(Snippet(path, line, textwrap.dedent(match.group("body"))))
    return found


def sdk_surface():
    import everruns_sdk

    # Constructing the client opens no connection; the dummy URL is never hit.
    client = everruns_sdk.Everruns(api_key="docs-snippet-check", base_url="http://127.0.0.1:9/api")
    return everruns_sdk, client


def signature_error(target, call: ast.Call) -> str | None:
    try:
        signature = inspect.signature(target)
    except (TypeError, ValueError):
        return None
    # A `**kwargs` signature accepts any keyword, so there is nothing to check.
    if any(p.kind == p.VAR_KEYWORD for p in signature.parameters.values()):
        return None
    if any(isinstance(arg, ast.Starred) for arg in call.args) or any(k.arg is None for k in call.keywords):
        return None
    try:
        signature.bind(*([None] * len(call.args)), **{k.arg: None for k in call.keywords})
    except TypeError as error:
        return str(error)
    return None


def check_snippet(snippet: Snippet, sdk, client) -> list[str]:
    try:
        tree = compile(
            snippet.source,
            str(snippet.path),
            "exec",
            flags=ast.PyCF_ONLY_AST | ast.PyCF_ALLOW_TOP_LEVEL_AWAIT,
        )
    except SyntaxError as error:
        return [f"{snippet.path}:{snippet.line + (error.lineno or 1) - 1}: does not parse: {error.msg}"]

    errors: list[str] = []
    sdk_names: dict[str, object] = {}
    for node in ast.walk(tree):
        if isinstance(node, ast.ImportFrom) and node.module == "everruns_sdk":
            for alias in node.names:
                if not hasattr(sdk, alias.name):
                    errors.append(f"{snippet.at(node)}: everruns_sdk has no `{alias.name}`")
                else:
                    sdk_names[alias.asname or alias.name] = getattr(sdk, alias.name)

    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        func = node.func
        if isinstance(func, ast.Name) and func.id in sdk_names and inspect.isclass(sdk_names[func.id]):
            problem = signature_error(sdk_names[func.id], node)
            if problem:
                errors.append(f"{snippet.at(node)}: `{func.id}(...)`: {problem}")
            continue
        if (
            isinstance(func, ast.Attribute)
            and isinstance(func.value, ast.Attribute)
            and isinstance(func.value.value, ast.Name)
            and func.value.value.id == "client"
        ):
            resource, method = func.value.attr, func.attr
            target = getattr(client, resource, None)
            if target is None:
                errors.append(f"{snippet.at(node)}: the client has no `{resource}` resource")
                continue
            bound = getattr(target, method, None)
            if bound is None or not callable(bound):
                errors.append(f"{snippet.at(node)}: `client.{resource}` has no `{method}` method")
                continue
            problem = signature_error(bound, node)
            if problem:
                errors.append(f"{snippet.at(node)}: `client.{resource}.{method}(...)`: {problem}")
    return errors


def run_program(snippet: Snippet) -> list[str]:
    with tempfile.NamedTemporaryFile("w", suffix=".py", delete=False) as handle:
        handle.write(snippet.source)
        script = handle.name
    print(f"Executing {snippet.where}", flush=True)
    try:
        result = subprocess.run(
            [sys.executable, script],
            env=os.environ.copy(),
            timeout=PROGRAM_TIMEOUT_SECONDS,
            capture_output=True,
            text=True,
        )
    except subprocess.TimeoutExpired:
        return [f"{snippet.where}: did not finish within {PROGRAM_TIMEOUT_SECONDS}s"]
    finally:
        os.unlink(script)
    sys.stdout.write(result.stdout)
    if result.returncode != 0:
        return [f"{snippet.where}: exited {result.returncode}\n{result.stderr}"]
    if "[failed" in result.stdout:
        return [f"{snippet.where}: the turn failed"]
    return []


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    repo_root = Path(__file__).resolve().parents[3]
    parser.add_argument("--docs", type=Path, default=repo_root / "docs")
    parser.add_argument("--execute", action="store_true", help="also run complete programs against EVERRUNS_API_URL")
    args = parser.parse_args()

    sdk, client = sdk_surface()
    found = snippets(args.docs)
    errors: list[str] = []
    for snippet in found:
        errors.extend(check_snippet(snippet, sdk, client))
    programs = [snippet for snippet in found if snippet.is_program]
    if args.execute:
        if not os.environ.get("EVERRUNS_API_URL"):
            errors.append("--execute needs EVERRUNS_API_URL")
        else:
            for program in programs:
                errors.extend(run_program(program))

    if errors:
        print("Python snippets do not match the SDK:", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        return 1
    executed = f", executed {len(programs)} program(s)" if args.execute else ""
    print(f"Checked {len(found)} Python block(s) against everruns-sdk{executed}.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
