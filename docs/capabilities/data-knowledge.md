---
title: Data Knowledge
description: Mount a read-only /knowledge/ Open Knowledge Format scaffold for table docs, business rules, and validated SQL, used by the Data Analyst harness.
appliesTo: [platform, cloud]
---

| | |
|---|---|
| **ID** | `data_knowledge` |
| **Category** | Data |
| **Tools** | None |
| **Features** | `file_system` |
| **Dependencies** | [File System](/capabilities/file-system/) |
| **Risk** | Low |

The `data_knowledge` capability mounts a read-only `/knowledge/` directory into
the session and tells the agent to read it before writing SQL. The directory is
shaped as an [Open Knowledge Format](/how-to/share-knowledge-with-okf/) (OKF)
bundle: Markdown files with YAML frontmatter, navigated through `index.md`
files.

The [Data Analyst](/built-ins/harnesses/data-analyst/) harness includes it.

## What gets mounted

```text
/knowledge/
  index.md              # OKF root index (okf_version "0.1")
  tables/index.md       # how to document a table or data source
  business/index.md     # how to document metrics and business rules
  queries/index.md      # how to document validated query patterns
```

Each `index.md` explains what belongs in its directory. The mount is a fixed,
read-only scaffold built into the capability: the agent cannot write to it,
and the capability has no configuration for adding your own files to it.

To give the agent your organization's curated data knowledge, put it in a
[Knowledge Base](/capabilities/knowledge-base/) (OKF bundles import directly)
or in an organization [Memory](/capabilities/memory/) mounted under
`/workspace`.

## System prompt

The capability adds one instruction: curated data knowledge is mounted at
`/knowledge/{tables,business,queries}`, and the agent should read it before
writing SQL and treat it as ground truth for schema semantics, metrics, and
validated queries.

## Configuration

None.

```json
{ "ref": "data_knowledge" }
```

## See also

- [Data Analyst harness](/built-ins/harnesses/data-analyst/)
- [Share knowledge with OKF](/how-to/share-knowledge-with-okf/)
- [Knowledge Base](/capabilities/knowledge-base/)
