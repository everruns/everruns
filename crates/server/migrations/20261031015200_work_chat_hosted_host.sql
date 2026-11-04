-- A Work Chat's public hostname is its own label. It is not the channel id.
-- New rows are assigned in create_agent_channel. This backfills channels that
-- already exist. The label stays when settings change; updates do not write it.
ALTER TABLE agent_channels
    ADD COLUMN hosted_host text;

CREATE UNIQUE INDEX agent_channels_hosted_host_idx
    ON agent_channels (hosted_host)
    WHERE hosted_host IS NOT NULL;

DO $$
DECLARE
    channel_id uuid;
    alphabet constant text := 'abcdefghijkmnpqrstuvwxyz23456789';
    label text;
    n int;
    i int;
    attempt int;
BEGIN
    FOR channel_id IN
        SELECT id FROM agent_channels
        WHERE channel_type = 'work_chat' AND hosted_host IS NULL
    LOOP
        attempt := 0;
        LOOP
            attempt := attempt + 1;
            IF attempt > 8 THEN
                RAISE EXCEPTION 'could not assign a Work Chat host';
            END IF;
            label := '';
            FOR i IN 1..10 LOOP
                n := 1 + floor(random() * length(alphabet))::int;
                label := label || substr(alphabet, n, 1);
            END LOOP;
            BEGIN
                UPDATE agent_channels SET hosted_host = label WHERE id = channel_id;
                EXIT;
            EXCEPTION
                WHEN unique_violation THEN
                    NULL;
            END;
        END LOOP;
    END LOOP;
END $$;

ALTER TABLE agent_channels
    ADD CONSTRAINT agent_channels_hosted_host_format
    CHECK (
        ((channel_type)::text = 'work_chat') = (hosted_host IS NOT NULL)
        AND (
            hosted_host IS NULL
            OR hosted_host ~ '^[abcdefghijkmnpqrstuvwxyz23456789]{10}$'
        )
    );

COMMENT ON COLUMN agent_channels.hosted_host IS
    'Public DNS label for a Work Chat. Assigned once, independent of public_id, and null for every other channel type.';
