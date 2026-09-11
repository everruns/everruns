# Agents (API)

* [TC001: Create Agent - Basic](TC001_create_agent_basic.md) - Verify that an agent can be created with required fields (name, system_prompt) plus optional display_name, and returns a valid agent object with both name and display_name.
* [TC002: Create Agent - With Capabilities](TC002_create_agent_with_capabilities.md) - Verify that an agent can be created with capabilities and that the capabilities are correctly stored and returned.
* [TC003: List Agents](TC003_list_agents.md) - Verify that agents are listed correctly, with pagination, and that archived agents are excluded by default.
* [TC004: Update Agent](TC004_update_agent.md) - Verify that an agent can be partially updated via PATCH, including name (slug) and display_name independently, and that only specified fields change.
* [TC005: Delete Agent - Archive and Hard Delete](TC005_delete_agent.md) - Verify the two-stage agent deletion: soft delete (archive) via DELETE, then hard delete via POST /delete.
* [TC006: Create Agent - Validation Errors](TC006_create_agent_validation.md) - Verify that agent creation fails with appropriate errors for missing, invalid, or duplicate fields, including addressable name format validation.
* [TC007: Check Agent Name Availability](TC007_check_agent_name.md) - Verify the `/v1/agents/check-name` endpoint correctly reports name availability, including format validation and exclude_id support for edit forms.
