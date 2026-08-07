# TC001: Knowledge Index Diagnostic Status

## Description

Verify that an active knowledge index with an invalid embedding model is not presented as healthy,
and that list and detail views show the same diagnostic with the correct remediation.

## Preconditions

- Server running with an organization selected
- An active knowledge index that has never synced and references a generation-only model

## Test Data

| Field | Value |
|-------|-------|
| Index ID | `kidx_019fda06ccf67d618776620713254657` when available |
| Embedding model | `Claude Opus 4.7 (1M)` or another model without `embeddings` capability |
| Sync status | `idle` |
| Last synced | unset |
| Documents | `0` |

## Steps

1. Navigate to `/knowledge-indexes`.
2. Locate the test index and inspect its lifecycle and diagnostic badges.
3. Verify the row does not offer a sync action while the embedding configuration is invalid.
4. Open the index detail page.
5. Inspect the prominent diagnostic notice and available action.
6. Open **Configure embedding model** and verify the picker does not offer generation-only models.

## Expected Result

| Check | Expected |
|-------|----------|
| Lifecycle | The index may show `active`, but also shows `Embedding model invalid` |
| Explanation | Copy names the selected model and states that it does not support embeddings |
| List action | **Configure embedding model** is visible; sync is not offered |
| Detail notice | A destructive, icon-labelled notice repeats the same diagnosis and explanation |
| Detail action | **Configure embedding model** opens the edit dialog |
| Picker | Only enabled models advertising `embeddings` are selectable |
| Empty state | The index is not labelled healthy or merely empty because it has never synced |
