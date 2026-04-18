#!/usr/bin/env python3

import json
from pathlib import Path

import nbformat
from nbclient import NotebookClient


def main() -> int:
    docs_app_root = Path(__file__).resolve().parent.parent
    manifest_path = docs_app_root / "src" / "generated" / "notebooks" / "manifest.json"
    repo_root = docs_app_root.parent.parent

    manifest = json.loads(manifest_path.read_text())
    notebook_paths = sorted(repo_root / entry["notebookFile"] for entry in manifest.values())

    if not notebook_paths:
        print("No notebook-backed docs pages found.")
        return 0

    for notebook_path in notebook_paths:
        print(f"Executing {notebook_path.relative_to(repo_root)}")
        notebook = nbformat.read(notebook_path, as_version=4)
        client = NotebookClient(
            notebook,
            timeout=600,
            kernel_name="python3",
            resources={"metadata": {"path": str(notebook_path.parent)}},
        )
        client.execute()

    print(f"Executed {len(notebook_paths)} notebook(s) successfully.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
