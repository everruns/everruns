#!/usr/bin/env python3

import json
import os
import uuid
from pathlib import Path

import nbformat
import requests
from nbclient import NotebookClient


def ensure_local_default_model() -> None:
    if os.environ.get("EVERRUNS_NOTEBOOK_USE_LLMSIM") != "1":
        return

    base_url = os.environ.get("EVERRUNS_API_URL")
    api_key = os.environ.get("EVERRUNS_API_KEY", "")
    if not base_url:
        raise RuntimeError("EVERRUNS_API_URL is required when EVERRUNS_NOTEBOOK_USE_LLMSIM=1")

    headers = {"Authorization": f"Bearer {api_key}"} if api_key else {}
    suffix = uuid.uuid4().hex[:8]

    provider = requests.post(
        f"{base_url}/v1/llm-providers",
        headers=headers,
        json={"name": f"Notebook LlmSim {suffix}", "provider_type": "llmsim"},
        timeout=30,
    )
    provider.raise_for_status()
    provider_id = provider.json()["id"]

    model = requests.post(
        f"{base_url}/v1/llm-providers/{provider_id}/models",
        headers=headers,
        json={
            "model_id": f"llmsim-notebook-{suffix}",
            "display_name": "Notebook LlmSim",
        },
        timeout=30,
    )
    model.raise_for_status()
    model_id = model.json()["id"]

    enable = requests.patch(
        f"{base_url}/v1/llm-models/{model_id}",
        headers=headers,
        json={"enabled": True},
        timeout=30,
    )
    enable.raise_for_status()

    orgs = requests.get(f"{base_url}/v1/orgs", headers=headers, timeout=30)
    orgs.raise_for_status()
    org_payload = orgs.json()
    org_list = org_payload.get("data", org_payload)
    if not org_list:
        raise RuntimeError("No organizations available for notebook execution")

    update = requests.patch(
        f"{base_url}/v1/orgs/{org_list[0]['id']}",
        headers=headers,
        json={"default_model_id": model_id},
        timeout=30,
    )
    update.raise_for_status()


def main() -> int:
    docs_app_root = Path(__file__).resolve().parent.parent
    manifest_path = docs_app_root / "src" / "generated" / "notebooks" / "manifest.json"
    repo_root = docs_app_root.parent.parent
    ensure_local_default_model()

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
