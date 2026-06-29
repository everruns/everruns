"""Thin client for the Everruns eval API (/v1/evals).

Kept self-contained (no SDK dependency) so this harness can run from a checkout
with only httpx installed. Mirrors the auth convention used by evals/swe-bench:
the API key is sent verbatim in the Authorization header (dev mode uses "dev";
personal access tokens should be passed as "Bearer evr_pat_...").
"""

from __future__ import annotations

import os
from typing import Any

import httpx


class EvalClient:
    def __init__(
        self,
        base_url: str | None = None,
        api_key: str | None = None,
        timeout: float = 60.0,
    ):
        base_url = (
            base_url or os.environ.get("EVERRUNS_API_URL", "http://localhost:9300/api")
        ).rstrip("/")
        api_key = api_key or os.environ.get("EVERRUNS_API_KEY", "dev")
        self._client = httpx.Client(
            base_url=base_url,
            headers={"Authorization": api_key, "Content-Type": "application/json"},
            timeout=timeout,
        )

    # -- Evals --
    def create_eval(
        self,
        name: str,
        *,
        description: str | None = None,
        target: dict | None = None,
        tags: list[str] | None = None,
    ) -> dict:
        body: dict[str, Any] = {"name": name}
        if description:
            body["description"] = description
        if target:
            body["target"] = target
        if tags:
            body["tags"] = tags
        return self._post("/v1/evals", body)

    def get_eval(self, eval_id: str) -> dict:
        return self._get(f"/v1/evals/{eval_id}")

    # -- Cases --
    def create_case(self, eval_id: str, case: dict) -> dict:
        return self._post(f"/v1/evals/{eval_id}/cases", case)

    def list_cases(self, eval_id: str) -> Any:
        return self._get(f"/v1/evals/{eval_id}/cases")

    # -- Runs --
    def create_run(
        self,
        eval_id: str,
        *,
        target: dict | None = None,
        model_override: str | None = None,
        filter_tags: list[str] | None = None,
    ) -> dict:
        body: dict[str, Any] = {}
        if target:
            body["target"] = target
        if model_override:
            body["model_override"] = model_override
        if filter_tags:
            body["filter_tags"] = filter_tags
        return self._post(f"/v1/evals/{eval_id}/runs", body)

    def get_run(self, eval_id: str, run_id: str) -> dict:
        return self._get(f"/v1/evals/{eval_id}/runs/{run_id}")

    def cancel_run(self, eval_id: str, run_id: str) -> dict:
        return self._post(f"/v1/evals/{eval_id}/runs/{run_id}/cancel", {})

    # -- HTTP helpers --
    def _get(self, path: str) -> Any:
        resp = self._client.get(path)
        resp.raise_for_status()
        return resp.json()

    def _post(self, path: str, body: dict) -> Any:
        resp = self._client.post(path, json=body)
        resp.raise_for_status()
        return resp.json()

    def close(self):
        self._client.close()
