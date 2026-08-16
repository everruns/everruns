---
title: Share knowledge with Open Knowledge Format (OKF)
description: Import and export Knowledge Bases as Open Knowledge Format bundles, portable markdown-with-frontmatter that any OKF consumer or agent can read, managed like code.
---

[Open Knowledge Format (OKF)](https://github.com/GoogleCloudPlatform/knowledge-catalog/tree/main/okf) is a vendor-neutral interchange format for the metadata and curated context around your data: a bundle is just a directory of markdown files with YAML frontmatter, shippable as a tarball or git repo. everruns Knowledge Bases speak OKF on both ends, so you can keep curated knowledge as code, ingest bundles produced elsewhere, and hand your agents' working set to any OKF consumer.

This guide covers importing a bundle into a Knowledge Base, the OKF↔entry mapping, and exporting a bundle back out.

## What a bundle looks like

```
sales/
├── index.md            # optional navigation (reserved file)
├── tables/
│   └── orders.md       # one concept document per file
└── metrics/
    └── revenue.md
```

A concept document is frontmatter plus markdown body:

```markdown
---
type: BigQuery Table
title: Orders
description: One row per completed customer order.
resource: https://console.cloud.google.com/bigquery?p=acme&d=sales&t=orders
tags: [sales, revenue]
---
# Schema
| Column | Type | Description |
|--------|------|-------------|
| `order_id` | STRING | Globally unique order identifier. |
```

Only `type` is required. `index.md` and `log.md` are reserved navigation/history files and never become entries.

## Import a bundle

`POST /v1/knowledge-bases/{kb_id}/okf_import` accepts either inline files or a base64-encoded `.tar.gz` bundle. It is **idempotent**: re-importing an updated bundle converges the Knowledge Base to the bundle's state without creating duplicates.

Create a Knowledge Base, then import inline files:

```bash
KB_ID=$(curl -s -X POST "http://localhost:9300/api/v1/knowledge-bases" \
  -H "Content-Type: application/json" \
  -d '{"name": "Sales Knowledge"}' | jq -r .id)

curl -s -X POST "http://localhost:9300/api/v1/knowledge-bases/$KB_ID/okf_import" \
  -H "Content-Type: application/json" \
  -d '{
    "files": [
      {
        "path": "tables/orders.md",
        "content": "---\ntype: BigQuery Table\ntitle: Orders\nresource: https://example.com/orders\ntags: [sales]\n---\n# Schema\nOne row per order.\n"
      }
    ]
  }'
```

Or import a tarball you already have:

```bash
curl -s -X POST "http://localhost:9300/api/v1/knowledge-bases/$KB_ID/okf_import" \
  -H "Content-Type: application/json" \
  -d "{\"bundle_base64\": \"$(base64 -w0 sales-bundle.tar.gz)\"}"
```

The response summarizes the run:

```json
{ "created": 1, "updated": 0, "skipped": 0, "pruned": 0, "warnings": [] }
```

### Keeping a Knowledge Base in sync

Re-run the same import whenever the bundle changes, matched entries update in place (keyed on `resource`, or the bundle path when there is no `resource`). To make the Knowledge Base a strict mirror of the bundle, pass `"prune": true`; entries that came from a previous import and are absent from the new bundle are removed.

```bash
curl -s -X POST "http://localhost:9300/api/v1/knowledge-bases/$KB_ID/okf_import" \
  -H "Content-Type: application/json" \
  -d "{\"prune\": true, \"bundle_base64\": \"$(base64 -w0 sales-bundle.tar.gz)\"}"
```

Import is tolerant by design (per OKF conformance): reserved files are skipped, and malformed documents are reported in `warnings` rather than failing the whole bundle.

## How OKF maps onto entries

| OKF frontmatter | Knowledge entry |
|-----------------|-----------------|
| `type` | `kind` (`note`/`table`/`business`/`query`/`runbook`); the raw `type` is preserved for faithful export |
| `title` | `title` (falls back to the filename) |
| `description` | folded into the body lead |
| `resource` | `resource` |
| `tags` | `tags` (lowercased) |
| markdown body | `body` |

`type` is matched case-insensitively: anything containing *table/dataset/view* → `table`, *metric/business/kpi/definition* → `business`, *query/sql* → `query`, *playbook/runbook/procedure* → `runbook`, otherwise `note`.

## Export a bundle

`GET /v1/knowledge-bases/{kb_id}/okf_export` streams a conformant OKF bundle as a gzipped tarball, including a root `index.md` that declares the format version.

```bash
curl -s "http://localhost:9300/api/v1/knowledge-bases/$KB_ID/okf_export" \
  -o sales-bundle.tar.gz
tar tzf sales-bundle.tar.gz
```

Export reconstructs each entry's frontmatter from the preserved raw `type` (or a default for its `kind`), its `resource`, tags, and timestamp, under a `kind`-derived directory. Export → import into a fresh Knowledge Base reproduces the same entries.

## Agents read OKF too

The `data_knowledge` capability mounts a readonly `/knowledge/` scaffold shaped as an OKF bundle (frontmatter + `index.md` navigation). Agents on the **Data Analyst** harness read it as ground truth before writing SQL. Import an OKF bundle into a Knowledge Base, and that curated context is available to your agents and portable to any other OKF consumer.

See also: [Data Analyst harness](/built-ins/harnesses/data-analyst/), and `knowledge/runtime-resources/okf-adoption.md` for the design intent.
