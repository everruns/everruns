ALTER TABLE session_participants
    ADD COLUMN display_name TEXT;

UPDATE session_participants AS participant
SET display_name = COALESCE(
    NULLIF(BTRIM(users.name), ''),
    NULLIF(BTRIM(principals.metadata ->> 'external_actor_name'), ''),
    NULLIF(BTRIM(principals.metadata ->> 'name'), ''),
    'User'
)
FROM principals
LEFT JOIN users ON users.id = principals.resolved_user_id
WHERE participant.kind = 'user'
  AND participant.principal_id = principals.id;

UPDATE session_participants
SET display_name = 'User'
WHERE kind = 'user' AND display_name IS NULL;

COMMENT ON COLUMN session_participants.display_name IS
    'Human-readable participant name captured from the authoritative identity profile or external actor.';
