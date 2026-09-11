# Agent instructions (API)

* [TC001: AGENTS.md Applied via Generic Harness](TC001_agents_md_applied_via_harness.md) - Verify that AGENTS.md instructions are automatically applied when using the Generic harness (which includes `agent_instructions` capability by default).
* [TC002: Missing AGENTS.md Silently Ignored](TC002_agents_md_missing_file_silent.md) - Verify that when no AGENTS.md file exists in the session filesystem, the agent operates normally without errors.
* [TC003: AGENTS.md Dynamic Update Between Turns](TC003_agents_md_dynamic_update.md) - Verify that changes to AGENTS.md are picked up on the next turn without restarting the session.
* [TC004: AGENTS.md Has No Effect with Base Harness](TC004_agents_md_base_harness_no_effect.md) - Verify that AGENTS.md is not read when using the Base harness, which does not include the `agent_instructions` capability.
* [TC005: AGENTS.md Size Limit (32 KiB)](TC005_agents_md_size_limit.md) - Verify that AGENTS.md content exceeding 32 KiB is truncated with a warning, not rejected.
* [TC006: Configured Instruction Files](TC006_configured_instruction_files.md) - Verify that `agent_instructions` keeps `AGENTS.md` as the default and reads additional configured instruction files such as `CLAUDE.md`.
