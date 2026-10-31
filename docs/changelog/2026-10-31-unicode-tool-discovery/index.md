---
title: Tool discovery recognizes multilingual words
description: Bounded tool discovery and tools search now match equivalent Unicode accents and words in Chinese, Japanese and Thai metadata within current access.
date: 2026-10-31
kind: fix
significance: minor
products: [platform, framework]
surfaces: [agent, code]
links:
  docs: [/capabilities/tool-search/]
---

Bounded tool discovery and `tools search` now find words inside uninterrupted
Chinese, Japanese and Thai text. Composed and decomposed accents match too:
`café` finds metadata written as `cafe` followed by a combining acute accent.

The shared Framework index uses the same normalization for metadata and queries
(`everruns-contracts` with the `runtime` feature):

```rust
use everruns_contracts::runtime::tool_catalog::ToolSearchIndex;
use everruns_contracts::tool_types::ToolDefinition;

fn find_warehouse(tools: &[ToolDefinition], allowed: impl Fn(&str) -> bool) -> Vec<String> {
    ToolSearchIndex::new(tools)
        .search_operations("仓库", 8, allowed)
        .into_iter()
        .map(|hit| hit.name)
        .collect()
}
```

For example, `仓库` matches a description containing `搜索仓库问题`. This does
not translate queries between languages. Exact names retain their ranking,
and results keep canonical invocation names and current access checks. See
[Tool Search](/capabilities/tool-search/) for bounded discovery configuration.
