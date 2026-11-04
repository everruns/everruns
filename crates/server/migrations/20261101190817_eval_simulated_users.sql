-- Multistep eval cases (knowledge/evaluation/multistep.md).
--
-- simulated_user: a model plays the user from a persona and a goal after the
-- scripted messages, until it says it is done or reaches its turn limit.
-- tool_fixtures: tools the case gives the agent with canned responses; the
-- runner answers each call and records it, so a case never reaches a real
-- system.
ALTER TABLE eval_cases
    ADD COLUMN simulated_user JSONB,
    ADD COLUMN tool_fixtures JSONB;
