"""Thin wrapper around Everruns eval API endpoints not yet in the SDK."""

from __future__ import annotations

import os
from typing import Any

import httpx


class EvalClient:
    """HTTP client for the /v1/evals endpoints."""

    def __init__(
        self,
        base_url: str | None = None,
        api_key: str | None = None,
        timeout: float = 60.0,
    ):
        base_url = (base_url or os.environ.get("EVERRUNS_API_URL", "http://localhost:9300/api")).rstrip("/")
        api_key = api_key or os.environ.get("EVERRUNS_API_KEY", "dev")
        self._client = httpx.Client(
            base_url=base_url,
            headers={"Authorization": api_key, "Content-Type": "application/json"},
            timeout=timeout,
        )

    # -- Evals --

    def create_eval(self, name: str, *, target: dict | None = None, tags: list[str] | None = None) -> dict:
        body: dict[str, Any] = {"name": name}
        if target:
            body["target"] = target
        if tags:
            body["tags"] = tags
        return self._post("/v1/evals", body)

    def get_eval(self, eval_id: str) -> dict:
        return self._get(f"/v1/evals/{eval_id}")

    def list_evals(self) -> list[dict]:
        return self._get("/v1/evals")

    # -- Cases --

    def create_case(self, eval_id: str, case: dict) -> dict:
        return self._post(f"/v1/evals/{eval_id}/cases", case)

    def list_cases(self, eval_id: str) -> list[dict]:
        return self._get(f"/v1/evals/{eval_id}/cases")

    # -- Runs --

    def create_run(self, eval_id: str, *, target: dict | None = None, model_override: str | None = None) -> dict:
        body: dict[str, Any] = {}
        if target:
            body["target"] = target
        if model_override:
            body["model_override"] = model_override
        return self._post(f"/v1/evals/{eval_id}/runs", body)

    def get_run(self, eval_id: str, run_id: str) -> dict:
        return self._get(f"/v1/evals/{eval_id}/runs/{run_id}")

    def list_runs(self, eval_id: str) -> list[dict]:
        return self._get(f"/v1/evals/{eval_id}/runs")

    def get_run_artifacts_ndjson(self, eval_id: str, run_id: str) -> str:
        resp = self._client.get(
            f"/v1/evals/{eval_id}/runs/{run_id}/artifacts",
            headers={"Accept": "application/x-ndjson"},
        )
        resp.raise_for_status()
        return resp.text

    def cancel_run(self, eval_id: str, run_id: str) -> dict:
        return self._post(f"/v1/evals/{eval_id}/runs/{run_id}/cancel", {})

    # -- Score write-back --

    def update_result_scores(
        self,
        eval_id: str,
        run_id: str,
        result_id: str,
        *,
        scores: list[dict],
        status: str | None = None,
        metadata: dict | None = None,
    ) -> dict:
        body: dict[str, Any] = {"scores": scores}
        if status:
            body["status"] = status
        if metadata:
            body["metadata"] = metadata
        return self._patch(f"/v1/evals/{eval_id}/runs/{run_id}/results/{result_id}/scores", body)

    def bulk_update_scores(
        self,
        eval_id: str,
        run_id: str,
        *,
        results: list[dict],
        metadata: dict | None = None,
    ) -> dict:
        body: dict[str, Any] = {"results": results}
        if metadata:
            body["metadata"] = metadata
        return self._patch(f"/v1/evals/{eval_id}/runs/{run_id}/scores", body)

    # -- HTTP helpers --

    def _get(self, path: str) -> Any:
        resp = self._client.get(path)
        resp.raise_for_status()
        return resp.json()

    def _post(self, path: str, body: dict) -> Any:
        resp = self._client.post(path, json=body)
        resp.raise_for_status()
        return resp.json()

    def _patch(self, path: str, body: dict) -> Any:
        resp = self._client.patch(path, json=body)
        resp.raise_for_status()
        return resp.json()

    def close(self):
        self._client.close()
