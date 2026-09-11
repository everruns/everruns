# Global search (API)

* [TC001: Global Search - Basic Query](TC001_search_basic.md) - Verify that the `?search=` parameter on entity list endpoints performs case-insensitive substring matching and returns only matching results.
* [TC002: Global Search - Multi-Word Tokenized Search](TC002_search_multiword.md) - Verify that multi-word search requires all tokens to match (AND semantics) and matches across name + description fields.
* [TC003: Global Search - Edge Cases and Robustness](TC003_search_edge_cases.md) - Verify that search handles adversarial and unusual inputs without crashing or performance degradation: poems, special characters, SQL wildcards, unicode, emoji, very long queries.
