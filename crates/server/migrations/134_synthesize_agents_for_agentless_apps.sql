-- Give grandfathered Apps an equivalent Agent before App-owned endpoints move
-- to Agent ownership.

CREATE TEMPORARY TABLE synthesized_app_agents ON COMMIT DROP AS
SELECT
    app.id AS app_id,
    app.public_id AS app_public_id,
    app.org_id,
    app.name AS app_name,
    app.harness_id,
    uuidv7() AS agent_id
FROM apps AS app
WHERE app.agent_id IS NULL;

INSERT INTO agents (
    id,
    org_id,
    public_id,
    name,
    display_name,
    system_prompt,
    harness_id,
    tags
)
SELECT
    synthesized.agent_id,
    synthesized.org_id,
    'agent_' || replace(synthesized.agent_id::text, '-', ''),
    left(
        COALESCE(
            NULLIF(
                trim(
                    BOTH '-' FROM regexp_replace(
                        regexp_replace(lower(synthesized.app_name), '[^a-z0-9-]', '-', 'g'),
                        '-+',
                        '-',
                        'g'
                    )
                ),
                ''
            ),
            'app'
        ),
        216
    ) || '-agent-' || replace(synthesized.app_id::text, '-', ''),
    synthesized.app_name || ' Agent',
    '',
    synthesized.harness_id,
    ARRAY[
        'synthesized-from-app',
        'synthesized-from-app:' || synthesized.app_public_id
    ]
FROM synthesized_app_agents AS synthesized;

UPDATE apps AS app
SET agent_id = synthesized.agent_id
FROM synthesized_app_agents AS synthesized
WHERE app.id = synthesized.app_id;

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM apps WHERE agent_id IS NULL) THEN
        RAISE EXCEPTION 'agent synthesis left Apps without an Agent';
    END IF;
END;
$$;
