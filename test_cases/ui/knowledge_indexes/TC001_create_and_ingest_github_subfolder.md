# TC001: Create and ingest a GitHub subfolder

## Description

Verify that creating a knowledge index queues its initial sync, ingests documents
from a GitHub subfolder, reports live state and counts, exposes failures, and can
be retried without changing the configured model.

## Preconditions

- API and background workers running with `just start-dev --no-watch`
- An active OpenAI provider with an enabled embeddings model
- For private repositories, a GitHub user connection

## Test Data

| Field | Value |
|---|---|
| Name | `Bashkit Knowledge Smoke <timestamp>` |
| Repository | `everruns/bashkit` |
| Branch | `main` |
| Root folder | `knowledge` |
| Embedding model | Any enabled model advertising the `embeddings` capability |

## Steps

1. Open **Knowledge Indexes → New index**. Verify only embedding-capable models
   appear. If none exist, verify the form links to the Models page and cannot be
   submitted.
2. Create an index using `everruns/bashkit`, branch `main`, root folder
   `knowledge`, and an enabled embedding model.
3. Observe the detail page without manually pressing **Sync now**.
4. Wait for the state to advance from `pending` to `syncing` and then `synced`.
5. Verify Documents and Chunks become non-zero and listed document source URIs
   begin with `github://everruns/bashkit@main/` and are relative to `knowledge`.
6. Edit the source to a nonexistent root folder. Save and wait for the sync.
7. Verify `failed` and a sanitized failure detail appear. Restore `knowledge`,
   save, and verify the automatic retry returns to `synced`.
8. Attempt API creation with an enabled chat-only model from an embeddings-capable
   provider. Verify HTTP 400 with the generic incompatible-model error.

## Expected Result

| Check | Expected |
|-------|----------|
| Initial state | Creation response is `pending`; no second sync request is needed |
| Live refresh | State and document counts update without reloading the page |
| Subfolder | Documents come from the configured `knowledge` folder |
| Failure | Sanitized detail is visible and existing indexed content is preserved |
| Retry | Source/model repair queues a new sync; manual retry is idempotent |
| Model safety | Chat, disabled, cross-org, and unsupported models are rejected uniformly |
