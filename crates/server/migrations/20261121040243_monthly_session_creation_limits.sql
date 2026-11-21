ALTER TABLE organization_settings
    ADD COLUMN monthly_session_limit integer NOT NULL DEFAULT 10000
    CHECK (monthly_session_limit BETWEEN 1 AND 100000);

-- Keep creation counts independent of session retention and serialize competing
-- inserts per organization and UTC month. Deleting a session never refunds it.
CREATE TABLE monthly_session_creations (
    org_id bigint NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    month_start date NOT NULL,
    sessions_created bigint NOT NULL,
    PRIMARY KEY (org_id, month_start)
);

-- THREAT[TM-DOS-024]: Count in the session insert transaction, including while
-- old and new server versions overlap during a rollout.
CREATE FUNCTION count_monthly_session_creation() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    session_limit integer;
    session_month date := date_trunc('month', NEW.created_at AT TIME ZONE 'UTC')::date;
BEGIN
    SELECT monthly_session_limit INTO session_limit
    FROM organization_settings WHERE org_id = NEW.org_id FOR SHARE;
    session_limit := COALESCE(session_limit, 10000);

    INSERT INTO monthly_session_creations (org_id, month_start, sessions_created)
    VALUES (NEW.org_id, session_month, 1)
    ON CONFLICT (org_id, month_start) DO UPDATE
        SET sessions_created = monthly_session_creations.sessions_created + 1
        WHERE monthly_session_creations.sessions_created < session_limit;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'Monthly session limit reached (max % per UTC calendar month)', session_limit
            USING ERRCODE = 'check_violation', CONSTRAINT = 'session_monthly_creation_limit';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER count_monthly_session_creation
    AFTER INSERT ON sessions
    FOR EACH ROW EXECUTE FUNCTION count_monthly_session_creation();

-- Creating the trigger holds a lock against inserts until the migration commits,
-- so existing current-month sessions are counted exactly once. Read the clock
-- after acquiring that lock in case waiting for it crosses a month boundary.
WITH current_month AS (
    SELECT date_trunc('month', clock_timestamp() AT TIME ZONE 'UTC') AS start_utc
)
INSERT INTO monthly_session_creations (org_id, month_start, sessions_created)
SELECT sessions.org_id, current_month.start_utc::date, count(*)
FROM sessions CROSS JOIN current_month
WHERE sessions.created_at >= current_month.start_utc AT TIME ZONE 'UTC'
  AND sessions.created_at < (current_month.start_utc + interval '1 month') AT TIME ZONE 'UTC'
GROUP BY sessions.org_id, current_month.start_utc;
