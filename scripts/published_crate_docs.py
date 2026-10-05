"""Validate public prose and extract crate documentation for the docs guard."""

import pathlib
import re


def crate_rustdoc(source: str, readme: str) -> tuple[bool, str]:
    lines = []
    for line in source.splitlines():
        if line.startswith("//!"):
            lines.append(line)
        elif re.fullmatch(r'#!\[doc\s*=\s*include_str!\("\.\./README\.md"\)\]\s*', line):
            # The canonical package README is already loaded and validated by
            # the caller. Treat its contents like literal docs without reading
            # arbitrary include paths from source.
            lines.extend(f"//! {text}" for text in readme.splitlines())
        elif not line.strip() or re.fullmatch(r"#!\[[^\n]*\]\s*", line):
            continue
        else:
            break
    return bool(lines), "\n".join(lines)


def file_reference_errors(root: pathlib.Path, text: str) -> list[tuple[int, str]]:
    """Public prose should stand alone; useful source references must be links."""
    errors = []
    fence = None
    for number, line in enumerate(text.splitlines(), 1):
        marker = re.match(r"^\s*(`{3,}|~{3,})", line)
        if marker:
            token = marker.group(1)
            if fence is None:
                fence = token
            elif token[0] == fence[0] and len(token) >= len(fence):
                fence = None
            continue
        if fence is not None:
            continue
        if re.search(r"(?<![\w/])knowledge/[\w./-]+\.md", line):
            errors.append((number, "remove internal knowledge references from public prose"))
        # Linked examples and schemas are useful. Inline commands and runtime
        # paths describe things the reader creates, not files to browse.
        prose = re.sub(r"!?\[[^\]]*\]\([^)]*\)", "", line)
        prose = re.sub(
            r"`([^`]*)`",
            lambda span: "" if re.search(r"\s", span.group(1)) else span.group(0),
            prose,
        )
        for match in re.finditer(r"(?<![\w/])(?:crates|apps|integrations|scripts|examples)/[\w./-]+", prose):
            path = match.group(0).rstrip(".,;")
            if (root / path).exists():
                errors.append((number, f"remove unnecessary source path or link it: {path}"))

    return errors
