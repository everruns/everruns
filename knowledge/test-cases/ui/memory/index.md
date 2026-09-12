# Memory (UI)

* [TC001: Create Memory](TC001_create_memory.md) - Verify that a new workspace memory can be created from the Memory page and appears in the list with `active` status and a `mem_` prefixed ID.
* [TC002: Create Memory - Required Name Validation](TC002_create_memory_validation.md) - Verify that the New Memory dialog rejects a whitespace-only name with an inline field error and does not create a memory.
* [TC003: Edit Memory Name and Description](TC003_edit_memory.md) - Verify that an existing active memory's name and description can be edited from the memory card and that changes are reflected in both the list and detail views.
* [TC004: Archive Memory and Toggle Archived Filter](TC004_archive_and_filter_memory.md) - Verify that a memory can be archived from the list, that archived memory are hidden by default, and that the archive filter toggle reveals them with a read-only `archived` badge and disabled write actions.
* [TC005: Search Memory by Name](TC005_search_memory.md) - Verify that the search input on the Memory page filters the list by name and surfaces the empty state when no memory match.
* [TC006: Memory Capability Mount Editor](TC006_memory_capability_config.md) - Verify that the **Memory** capability config editor (on an agent or harness) lets users add, configure, and remove memory mounts, and surfaces inline validation for invalid paths, duplicate paths, and overlapping paths.
* [TC007 Scoped agent and user memory](TC007_scoped_agent_and_user_memory.md) - Verify that host-agent memory is mounted at `/memory/agent` and persists across two sessions of the same agent, while user memory is mounted only in sessions where that user participates.
