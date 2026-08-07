# TC001: OKF Import / Export Round-Trip

## Description

Verify a Knowledge Base round-trips through Open Knowledge Format (OKF): import a
bundle of concept documents, confirm entries are created with the correct
`kind`/`resource`/`tags`, confirm a re-import is idempotent (no duplicates),
then export and confirm the bundle reproduces the documents.

Exercises `POST /v1/knowledge-bases/{kb_id}/okf_import` and
`GET /v1/knowledge-bases/{kb_id}/okf_export`. See knowledge/runtime-resources/okf-adoption.md.

## Preconditions

- Control-plane running (`just start-dev` or `just start-all`)
- `jq` and `tar` available

## Test Data

A two-document OKF bundle (a table and a metric):

| Path | type | resource |
|------|------|----------|
| `tables/orders.md` | `BigQuery Table` | `https://example.com/orders` |
| `metrics/active_users.md` | `Metric` | (none) |

## Steps

1. **Create a Knowledge Base:**
   ```bash
   KB_ID=$(curl -s -X POST "http://localhost:9300/api/v1/knowledge-bases" \
     -H "Content-Type: application/json" \
     -d '{"name": "OKF Round-Trip"}' | jq -r .id)
   echo "$KB_ID"   # expect kb_...
   ```

2. **Import the bundle (inline files):**
   ```bash
   curl -s -X POST "http://localhost:9300/api/v1/knowledge-bases/$KB_ID/okf_import" \
     -H "Content-Type: application/json" \
     -d '{
       "files": [
         { "path": "tables/orders.md",
           "content": "---\ntype: BigQuery Table\ntitle: Orders\nresource: https://example.com/orders\ntags: [sales]\n---\n# Schema\nOne row per order.\n" },
         { "path": "metrics/active_users.md",
           "content": "---\ntype: Metric\ntitle: Active Users\n---\nLogged in within 30 days.\n" }
       ]
     }' | jq .
   ```

3. **List entries:**
   ```bash
   curl -s "http://localhost:9300/api/v1/knowledge-bases/$KB_ID/entries" | jq '.items[] | {title, kind, resource, tags}'
   ```

4. **Re-import the identical bundle** (repeat step 2) and inspect the summary.

5. **Export the bundle:**
   ```bash
   curl -s "http://localhost:9300/api/v1/knowledge-bases/$KB_ID/okf_export" -o /tmp/okf-export.tar.gz
   tar tzf /tmp/okf-export.tar.gz
   tar xzf /tmp/okf-export.tar.gz -O tables/orders.md
   ```

## Expected Result

### Import (step 2)
- Response: `{ "created": 2, "updated": 0, "skipped": 0, "pruned": 0, "warnings": [] }`.

### Entries (step 3)
- Two entries. `Orders` has `kind: "table"`, `resource: "https://example.com/orders"`, `tags` includes `sales`.
- `Active Users` has `kind: "business"` (Metric → business), no `resource`.

### Re-import (step 4)
- Response: `{ "created": 0, "updated": 2, ... }` — idempotent, no new entries created. Step 3 still lists exactly two entries.

### Export (step 5)
- The tarball contains `index.md`, `tables/orders.md`, and `metrics/active_users.md`.
- `index.md` frontmatter contains `okf_version: "0.1"`.
- `tables/orders.md` frontmatter reproduces `type: BigQuery Table`, `title: Orders`, and `resource: https://example.com/orders`.

### Failure Modes

| Failure | What to look for |
|---------|-----------------|
| Duplicates on re-import | `created` > 0 on step 4, or step 3 shows > 2 entries |
| Wrong kind mapping | `Metric` not mapped to `business`, or `BigQuery Table` not to `table` |
| Lost resource/tags | `resource` or `tags` missing on the `Orders` entry |
| Type not preserved | exported `tables/orders.md` shows a generic `type` instead of `BigQuery Table` |

## Notes

- Reserved files (`index.md`, `log.md`) in an imported bundle never create entries.
- To make the KB a strict mirror of a bundle, pass `"prune": true` on import.
