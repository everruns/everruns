-- Release an agent's default model when the model is deleted.
--
-- `agents.default_model_id` was NO ACTION, so deleting a model that any agent
-- had ever selected as its default was refused by
-- `agents_default_model_id_fkey`. Nothing could release the reference either:
-- `DELETE /v1/agents/{id}` and `POST /v1/agents/{id}/delete` are both soft
-- deletes (`status = 'deleted'`, the row stays), and `PATCH /v1/agents/{id}`
-- takes `default_model_id` as a plain `Option`, where null is indistinguishable
-- from "leave unchanged". A model picked as an agent default was therefore
-- undeletable for the life of the database (EVE-955).
--
-- SET NULL matches what `organization_settings.default_model_id` has done since
-- 007: the model goes, and the agent falls back to the org/harness default
-- instead of pinning a row nobody can remove. The previous behaviour was not a
-- useful guard — it surfaced as an opaque 500 with no indication of which agent
-- held the reference.
--
-- `sessions.model_id` is deliberately left alone: it records which model a
-- session actually ran on, and nulling that would rewrite history. Callers
-- delete their sessions instead.
ALTER TABLE agents
    DROP CONSTRAINT agents_default_model_id_fkey;

ALTER TABLE agents
    ADD CONSTRAINT agents_default_model_id_fkey
    FOREIGN KEY (default_model_id) REFERENCES models(id) ON DELETE SET NULL;
