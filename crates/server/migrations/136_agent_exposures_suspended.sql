-- Phase 6 of retiring the App abstraction (EVE-1007): per-endpoint publish plus
-- one agent-level incident switch.
--
--   live(endpoint) = endpoint.status == 'live'
--                 && agent.status == 'active'
--                 && !agent.exposures_suspended
--
-- `agent_endpoints.status` already exists (135) and carried a derived value;
-- this is the migration that makes it authoritative at ingress, so the derived
-- value has to become the real one for every existing row first.
--
-- Agent status is deliberately NOT denormalized onto the endpoint. It is folded
-- in at resolution time, the same way the harness overlay chain is: writing rows
-- when an agent is archived would make the archive un-restorable and create a
-- second writer for endpoint status.

-- The incident control: one switch that takes an agent off the internet without
-- touching the per-endpoint state it should restore to.
ALTER TABLE agents
    ADD COLUMN exposures_suspended BOOLEAN NOT NULL DEFAULT false;

-- Until now the effective gate was `app.status = 'published' AND channel.enabled`.
-- 135 backfilled `status` from exactly that pair, and the endpoint update path
-- has kept it in step since, so the values already agree. Re-assert it here
-- rather than trusting that: from the next commit on, this column alone decides
-- whether traffic is accepted, and a row that disagrees would silently either
-- expose or hide an endpoint.
UPDATE agent_endpoints AS ae
SET status = CASE
        WHEN NOT ae.enabled THEN 'disabled'
        WHEN app.status = 'published' THEN 'live'
        ELSE 'draft'
    END
FROM apps AS app
WHERE app.id = ae.app_id
  AND ae.status IS DISTINCT FROM CASE
        WHEN NOT ae.enabled THEN 'disabled'
        WHEN app.status = 'published' THEN 'live'
        ELSE 'draft'
    END;

DO $$
DECLARE
    mismatched BIGINT;
BEGIN
    SELECT COUNT(*)
    INTO mismatched
    FROM agent_endpoints AS ae
    JOIN apps AS app ON app.id = ae.app_id
    WHERE ae.status <> CASE
            WHEN NOT ae.enabled THEN 'disabled'
            WHEN app.status = 'published' THEN 'live'
            ELSE 'draft'
        END;

    IF mismatched > 0 THEN
        RAISE EXCEPTION
            'endpoint status disagrees with the App publish state it replaces for % row(s); ingress would change behavior for them',
            mismatched;
    END IF;
END;
$$;

-- Ingress resolves an endpoint by public_id and then needs the owning agent's
-- status and suspend flag on the same request.
CREATE INDEX idx_agent_endpoints_status ON agent_endpoints(status);
