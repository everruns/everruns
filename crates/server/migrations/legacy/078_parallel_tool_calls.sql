-- Add request-level parallel tool calling preference to agents and sessions (EVE-598).
--
-- `parallel_tool_calls` mirrors the request-level signal sent to the LLM
-- provider. A NULL value (the default for all existing rows) preserves current
-- behavior: the provider default is used and the act scheduler keeps its
-- class-aware concurrent schedule. `TRUE` explicitly signals the provider that
-- parallel tool calls are wanted; `FALSE` requests at most one tool call per
-- turn and forces serial execution.
--
-- Nullable with no default so absence stays distinguishable from an explicit
-- choice, matching how `max_iterations` is stored. No backfill is required.
ALTER TABLE agents ADD COLUMN parallel_tool_calls BOOLEAN;
ALTER TABLE sessions ADD COLUMN parallel_tool_calls BOOLEAN;
