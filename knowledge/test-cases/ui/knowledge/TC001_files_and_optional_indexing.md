---
type: Test Case
title: "TC001: Knowledge - Files and optional indexing"
description: "Verify that one Knowledge collection supports notes, binary originals, extracted text, optional search and file visibility controls."
tags:
  - ui
  - knowledge
---
# TC001: Knowledge - Files and optional indexing

## Description

Verify that one Knowledge collection supports notes, binary originals, extracted text, optional search and file visibility controls.

## Preconditions

- Local API, worker and UI are running with `AUTH_MODE=none`, or a signed-in organization admin.
- Development deployment with the organization's Knowledge feature enabled.
- Use an isolated test collection. No external indexer or model key is needed for keyword mode.

## Test Data

| Field | Value |
| --- | --- |
| Name | Knowledge acceptance |
| Note path | policies/refunds.md |
| Note | Markdown with OKF frontmatter and a thirty-day refund policy |
| Binary | `crates/server/testdata/knowledge/minimal.docx` |
| Search | Refund |

## Steps

1. Open `/knowledge`. Verify predefined `everruns-docs` appears without connecting credentials.
2. Create the collection with indexing off. Open it and save the note.
3. Upload the DOCX. Select both files and inspect their text views and metadata.
4. Download the DOCX original and compare its bytes with the input.
5. In collection settings choose keyword search and save. Wait for processing; search for Refund.
6. Verify cited results include paths and immutable version IDs.
7. Select the note and disable Include in index. Search again after processing; verify it is absent while the original remains in the catalog.
8. Disable Agent can read, then verify an attached session cannot list, read or grep it.
9. Reload the page and verify file policy persists. Open `/knowledge-indexes` and verify it redirects to `/knowledge`.
10. Use **Upload folder** with an OKF directory containing root `index.md`, `log.md`, a concept with custom frontmatter and a binary asset. Verify the directory-relative paths and bytes survive. Validate its metadata through the paginated OKF endpoint with that folder as the path prefix.
11. Attach two sessions to one workspace with different Knowledge bindings. Verify only the attached session can read/grep this collection and both see the same ordinary workspace files.

## Expected Result

Notes and binaries share one file catalog. Indexing is optional. DOCX extraction uses Markkit; originals round-trip unchanged. Indexed search returns immutable citations and enforces live file policy. Legacy navigation leads to the single Knowledge feature.
