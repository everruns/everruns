-- Agents use a monthly creation quota, like sessions: a lifetime cap on live
-- agents stopped short-lived agents (evaluation, deployment probes) once
-- archived ones piled up. Built-in agents never count.
ALTER TABLE organization_settings
    ADD COLUMN monthly_agent_limit integer NOT NULL DEFAULT 500
    CHECK (monthly_agent_limit BETWEEN 1 AND 100000);

-- Keep creation counts independent of agent retention and serialize competing
-- inserts per organization and UTC month. Archiving or deleting never refunds.
CREATE TABLE monthly_agent_creations (
    org_id bigint NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    month_start date NOT NULL,
    agents_created bigint NOT NULL,
    PRIMARY KEY (org_id, month_start)
);

-- THREAT[TM-DOS-024]: Count in the agent insert transaction, including while
-- old and new server versions overlap during a rollout. SECURITY DEFINER so
-- restricted writers consume quota without access to settings or counters.
CREATE FUNCTION public.count_monthly_agent_creation() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path TO pg_catalog, public, pg_temp AS $$
DECLARE
    agent_limit integer;
    agent_month date := date_trunc('month', NEW.created_at AT TIME ZONE 'UTC')::date;
BEGIN
    SELECT monthly_agent_limit INTO agent_limit
    FROM organization_settings WHERE org_id = NEW.org_id FOR SHARE;
    agent_limit := COALESCE(agent_limit, 500);

    INSERT INTO monthly_agent_creations (org_id, month_start, agents_created)
    VALUES (NEW.org_id, agent_month, 1)
    ON CONFLICT (org_id, month_start) DO UPDATE
        SET agents_created = monthly_agent_creations.agents_created + 1
        WHERE monthly_agent_creations.agents_created < agent_limit;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'Monthly agent limit reached (max % per UTC calendar month)', agent_limit
            USING ERRCODE = 'check_violation', CONSTRAINT = 'agent_monthly_creation_limit';
    END IF;
    RETURN NEW;
END;
$$;

REVOKE EXECUTE ON FUNCTION public.count_monthly_agent_creation() FROM PUBLIC;

CREATE TRIGGER count_monthly_agent_creation
    AFTER INSERT ON agents
    FOR EACH ROW WHEN (NOT NEW.is_built_in)
    EXECUTE FUNCTION count_monthly_agent_creation();

-- Creating the trigger holds a lock against inserts until the migration commits,
-- so existing current-month agents are counted exactly once. Read the clock
-- after acquiring that lock in case waiting for it crosses a month boundary.
WITH current_month AS (
    SELECT date_trunc('month', clock_timestamp() AT TIME ZONE 'UTC') AS start_utc
)
INSERT INTO monthly_agent_creations (org_id, month_start, agents_created)
SELECT agents.org_id, current_month.start_utc::date, count(*)
FROM agents CROSS JOIN current_month
WHERE NOT agents.is_built_in
  AND agents.created_at >= current_month.start_utc AT TIME ZONE 'UTC'
  AND agents.created_at < (current_month.start_utc + interval '1 month') AT TIME ZONE 'UTC'
GROUP BY agents.org_id, current_month.start_utc;
