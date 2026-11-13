---
title: Typed Extractions
description: Extract typed values and evidence from documents without an agent loop.
---

Extractions ask for named fields in text or images and return typed values, evidence and uncertainty. They are separate from Decisions (classification) and structured chat (schema-shaped generated output). The Framework exposes the same contract as the Server. No agent, session or tool loop is required.

The upstream OpenRouter API is **alpha**. Providers differ in supported fields and assets; an unsupported request fails rather than silently switching to chat or inventing confidence.

## Framework

```rust
use everruns::{Extractions, ExtractionField};
async fn example() -> Result<(), Box<dyn std::error::Error>> {
let key = std::env::var("OPENROUTER_API_KEY")?;
let provider = everruns_drivers::openrouter::provider("openrouter", key);
let result = Extractions::new("cloudflare/clairic", provider)
    .about("Invoice INV-42. Total USD 120.00.")
    .field("invoice_number", ExtractionField::text())
    .field("total", ExtractionField::number())
    .send().await?;
println!("{:?}", result.answers);
    Ok(())
}
```

Enable the driver's `openrouter` feature. Run the synthetic invoice example:

```bash
OPENROUTER_API_KEY=... cargo run -p everruns --features utility-llm --example direct_extractions
```

This sends a paid provider request. Set `EXTRACTION_MODEL` to another extraction model supported by your account. Do not use real private documents in tests without authorizing their transmission.

For direct Cloudflare access, construct `everruns_drivers::cloudflare::provider` with your account ID and API token and select `@cf/cloudflare/clairic`. It uses Workers AI directly, without an OpenRouter credential. The Cloudflare driver accepts text and images, not PDFs. Run `cargo run -p everruns --features cloudflare --example direct_cloudflare_extractions` with `CLOUDFLARE_ACCOUNT_ID` and `CLOUDFLARE_API_TOKEN` to try the synthetic invoice directly (provider charges may apply). Other upstream vendors are available through OpenRouter; they are not advertised as direct integrations here.

## Server and console

Configure an organization-owned provider key and add or sync an extraction-capable model. Open **Extractions** in the console to supply text and field specifications, then inspect values and evidence. Inline assets are supported through the API; the initial console does not include an asset upload widget.

```bash
curl "$EVERRUNS_URL/v1/extractions" \
  -H "Authorization: Bearer $EVERRUNS_API_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"model_id":"YOUR_MODEL_ID","input":{"state":"Invoice INV-42","fields":{"invoice_number":{"type":"text"}}}}'
```

The API requires model-management permission and an enabled extraction model in the authenticated organization. Read-only credentials cannot make paid calls. This initial endpoint supports organization BYOK providers; hosted/billed models are rejected until their extraction accounting path is available. Model/provider overrides inside `input` are rejected. Credentials stay on the server. Requests carry no arbitrary remote document URL: assets are inline base64. OpenRouter transport additionally caps fields at 128, state at 2 MiB and encoded assets at 12 MiB as client-side safeguards, not vendor limit claims. The contract caps assets at 32 and their aggregate encoded data at 32 MiB before decoding. Driver-specific limits may be lower; the Server limits the whole request body to 8 MiB.

## Values, absence and evidence

Field types are `text`, `number`, `date`, `time`, `datetime`, `duration`, `list`, `estimate` and `forecast`. `ExtractionField` validates type-specific options before network access. The outcome preserves provider usage, `truncated`, optional state text and every returned field's value.

- `value: null` is a valid absent answer. Do not replace it with `guess` automatically.
- Confidence and `p_null` are optional provider observations, not calibrated guarantees. Missing confidence means unknown, not zero or one.
- `span` and per-item `spans` use Unicode **code points**, not byte or JavaScript UTF-16 offsets. In JavaScript use `Array.from(state).slice(start, end).join('')`. A list may have null span entries.
- Use returned `state_text` when present. Truncation means the model did not read the whole input.
- Estimates and forecasts preserve distribution statistics; they are not exact source facts.
- Image/document answers may have no textual span. Do not fabricate evidence.

## Deployment-owned utility

With the Framework's `utility-llm` feature, `utility_extractions::utility_extractions_from_env()` selects the explicit `UTILITY_EXTRACTION_MODEL` and `UTILITY_OPENROUTER_API_KEY`. With no extraction model configured, it is disabled. Tenant extraction endpoints never fall back to these deployment credentials.

## Document conversion and Markkit

Keep conversion separate from inference. Convert a local document to Markdown with Markkit, review the text, then pass that text as extraction state. This preserves Markkit's deterministic offline boundary and keeps provider credentials in Everruns. The example accepts a reviewed Markdown file from a separate converter:

```bash
EXTRACTION_INPUT_FILE=reviewed-invoice.md OPENROUTER_API_KEY=... \
  cargo run -p everruns --features utility-llm --example direct_extractions
```

This explicitly sends that file's text to the provider. For scanned documents, use a provider-supported asset input instead: conversion quality and OCR are not guaranteed by the extraction contract.

## Verification

Unit and mock-HTTP tests cover typed validation, malformed answers, Unicode spans, auth/tenant checks and sanitized errors. The live qualification harness is opt-in, uses synthetic documents and requires separate credentials and an explicit spending decision. A mock pass does not prove vendor availability or extraction accuracy. After explicitly approving provider spending and setting the provider credentials, run the synthetic-document live qualification harness:

```bash
EVERRUNS_EXTRACTIONS_LIVE=1 EXTRACTIONS_LIVE_PROVIDER=openrouter \
  cargo bench -p everruns-drivers \
  --no-default-features --features openrouter,cloudflare \
  --bench extractions_live
```

Select `openrouter` or `cloudflare` with `EXTRACTIONS_LIVE_PROVIDER`; do not supply credentials for a provider you do not intend to call.

See [OpenRouter's extraction contract](https://openrouter.ai/docs/guides/community/extractions) and [Cloudflare Clairic](https://developers.cloudflare.com/workers-ai/models/clairic/) for current limits.
