-- Link schedule-backed app channels to durable schedule rows.

ALTER TABLE app_channels
    ADD COLUMN durable_schedule_id UUID REFERENCES durable_schedules(id) ON DELETE SET NULL;

CREATE UNIQUE INDEX app_channels_durable_schedule_id_idx
    ON app_channels (durable_schedule_id)
    WHERE durable_schedule_id IS NOT NULL;
