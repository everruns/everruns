# Agent chat (UI)

* [TC001: Agent Chat - Multi-turn Conversation](TC001_multi_turn_agent_conversation.md) - Verify that a user can open a direct chat session with an agent, send a message, wait for the full streamed response, then send a follow-up message in the same session and receive a contextual response.
* [TC002: Tool Activity Narration and Replay](TC002_tool_activity_narration.md) - Verify that multi-tool activity renders once per execution batch with human-readable narration and stable state across live streaming, reload, and backward event pagination.
* [TC003: Chat edits the live Agent draft](TC003_page_draft_chat.md) - Verify unsaved form reads, staged Chat edits, guarded Undo, Save and Discard, and page-bound execution.
* [TC004: Draft conflicts and guarded Undo](TC004_draft_conflicts_and_undo.md) - Verify stale patches and Undo preserve later manual edits.
* [TC005: Page and organization binding](TC005_page_and_org_binding.md) - Verify navigation, organization switches and other browsers cannot reuse a page binding.
* [TC006: Draft permissions and saved-definition conflicts](TC006_draft_permissions_and_saved_conflicts.md) - Verify read-only, unauthorized, saving and conflicting forms refuse patches.
* [TC007: Chat workspace layout, persistence and rollout](TC007_chat_mode_layout_and_rollout.md) - Verify Adoption gating, persistent state, pane-scoped sheets and accessible responsive layouts.

* [TC008: Chat context across app pages](TC008_all_page_context.md) - Verify identity, live view state, scoped summaries and read-only action boundaries across every app area.
* [TC009: Chat edits shared page forms](TC009_shared_page_forms.md) - Verify staged edits, guarded Undo, independent forms, credentials and stable dialogs beyond Agents.
* [TC010: Chat stages settings before Save](TC010_staged_settings.md) - Verify staged settings, explicit Save/Discard, conflicts, permissions and legacy autosave.
* [TC011: Secondary forms and protected dialogs](TC011_secondary_forms.md) - Verify secondary drafts, refresh preservation, credential boundaries and dialog lifecycle.
* [TC012: Operational page context](TC012_operational_context.md) - Verify visible status projections, platform authorization and manual action boundaries.

* [TC013: Shared saved review and guarded Undo](TC013_shared_saved_review.md) - Verify diffs, saved inverses, concurrent edits, atomic batches and private account history.
