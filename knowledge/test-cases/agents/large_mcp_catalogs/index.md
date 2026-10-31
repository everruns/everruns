# Large MCP catalogs (Agents)

* [TC001: Paginated catalog discovery and search](TC001_paginated_catalog_discovery_and_search.md) - Verify every page of a large catalog is searchable and the selected operation can be called with its authoritative schema.
* [TC002: Authorized server metadata and colliding names](TC002_authorized_server_metadata_and_colliding_names.md) - Verify server context improves search while authorization and server-qualified operation identity remain enforced.
* [TC003: Native, CLI, and hybrid registry parity](TC003_native_cli_hybrid_registry_parity.md) - Verify all configured surfaces route to the same MCP operation and preserve its arguments and result.
* [TC004: Dynamic catalog changes and replay](TC004_dynamic_catalog_changes_and_replay.md) - Verify attachment, plugin, schema, authorization, restart, and compaction changes do not restore stale execution authority.
* [TC005: Discovery limits and incomplete catalogs](TC005_discovery_limits_and_incomplete_catalogs.md) - Verify exact bounds are accepted and over-limit or incomplete paginated catalogs fail without appearing complete.
* [TC006: Deferred loading limits and scope](TC006_deferred_server_loading_limits_and_scope.md) - Verify bounded deferred-server loading remains atomic and within the current turn's scope.
* [TC007: Sparse session context](TC007_sparse_session_context.md) - Verify selected schemas, CLI help, dynamic plugins, and usage accounting stay bounded and observable.
