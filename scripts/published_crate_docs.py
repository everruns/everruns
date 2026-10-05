"""Extract leading crate documentation for the published-package guard."""

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
