-- Stable global identity for the encryption rotation registry; tenant/key CAS stays unchanged.
ALTER TABLE dreaming_state
    ADD COLUMN id UUID NOT NULL DEFAULT gen_random_uuid(),
    ADD CONSTRAINT dreaming_state_rotation_identity UNIQUE (id);
