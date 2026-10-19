-- tighten_channel_type_drop_api_endpoint
--
-- Agent Execution API phase 6 removed the `api_endpoint` channel type: the
-- server converts every such channel to an `api` channel at startup
-- (crates/server/src/domains/agent_channels/api_endpoint_conversion.rs). This
-- drops 'api_endpoint' from the channel_type CHECK on `agent_channels` and
-- `apps`, so no write can bring the type back.
--
-- Decision: tighten a table only when it holds no 'api_endpoint' row; otherwise
-- keep its CHECK as it was and raise a WARNING. Not fail, and not convert here:
-- - Migrations run before the startup conversion, so a deployment upgrading
--   straight from a release older than phase 6 still has its channels
--   unconverted when this runs. Failing would block that upgrade.
-- - Converting in SQL would lose what only the startup conversion can do: the
--   key hash and auth sit in encrypted columns, and it imports the key as an
--   agent key and retags the channel's sessions. It also leaves a channel it
--   cannot decrypt untouched and retries it on the next start, which needs the
--   old type to stay writable.
-- - A NOT VALID constraint would admit the leftovers but reject any later
--   UPDATE of them that keeps the type (archiving, App publish), so it would
--   break those paths for exactly the rows still waiting.
-- A database that skipped the tightening keeps the old CHECK until a later
-- migration removes the conversion, which must then settle the leftovers.
-- `apps` holds archived App records, which nothing converts; the same rule
-- applies to it on its own.
-- The statements are idempotent, so this is safe to re-run.

DO $$
DECLARE
    leftovers BIGINT;
BEGIN
    SELECT count(*) INTO leftovers FROM agent_channels WHERE channel_type = 'api_endpoint';
    IF leftovers > 0 THEN
        RAISE WARNING 'agent_channels has % api_endpoint row(s); its channel_type CHECK keeps allowing api_endpoint until the startup conversion converts them', leftovers;
    ELSE
        ALTER TABLE agent_channels DROP CONSTRAINT IF EXISTS agent_channels_channel_type_check;
        ALTER TABLE agent_channels ADD CONSTRAINT agent_channels_channel_type_check
            CHECK (channel_type IN ('slack', 'ag_ui', 'schedule', 'webhook', 'a2a', 'fcp',
                                    'public_chat', 'voice', 'api', 'poppy'));
    END IF;

    SELECT count(*) INTO leftovers FROM apps WHERE channel_type = 'api_endpoint';
    IF leftovers > 0 THEN
        RAISE WARNING 'apps has % api_endpoint row(s); its channel_type CHECK keeps allowing api_endpoint', leftovers;
    ELSE
        ALTER TABLE apps DROP CONSTRAINT IF EXISTS apps_channel_type_check;
        ALTER TABLE apps ADD CONSTRAINT apps_channel_type_check
            CHECK (channel_type IN ('slack', 'ag_ui', 'schedule', 'webhook', 'a2a', 'fcp',
                                    'public_chat', 'voice', 'api', 'poppy'));
    END IF;
END $$;
