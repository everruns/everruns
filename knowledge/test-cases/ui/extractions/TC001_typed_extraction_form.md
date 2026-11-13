---
type: Test Case
title: "TC001: Typed extraction form"
description: "Verify typed extraction model selection, field validation and organization-isolated results."
tags: [everruns, test-case, ui, extractions]
---
# TC001: Typed extraction form

## Description

Verify typed extraction model selection, field validation and organization-isolated results.

## Preconditions

- Canonical local stack on a distinct port prefix.
- Model-management permission and an enabled organization-owned extraction model.
- A synthetic provider fixture for offline testing, or explicit permission for provider spending before a live call.

## Test Data

Synthetic source: `Invoice INV-42. Total USD 120.00.`
Fields: `{"invoice_number":{"type":"text"},"total":{"type":"number"}}`.

## Steps

1. Open Extractions through the sidebar and Command-K. Verify the charge warning, source and field inputs, and the extraction model selector in light and dark themes.
2. With no model or source, verify Extract is disabled. Choose an extraction model; chat-only models must not appear.
3. Enter malformed field JSON, then Extract. Verify a local error and no API request.
4. Enter valid fields and Extract. Verify inputs and button disable while pending and a second click cannot submit again.
5. Inspect the completed result: typed values, nullable evidence and missing confidence remain unchanged.
6. While another request is pending, switch organizations. Verify the form clears, and completing the old request cannot reveal its source or result.
7. Trigger a provider error. Verify the generic error, no credential/document leakage, and a usable retry button.
8. Navigate away and Back. Verify navigation works and an old request cannot replace a newly submitted result.

## Expected Result

- Only enabled extraction-capable models are offered.
- Invalid JSON never causes provider spending; pending interactions do not duplicate calls.
- Results retain provider evidence without fabricating confidence.
- Form and result state never cross organization boundaries.
