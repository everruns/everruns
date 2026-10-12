-- Notifications name what sent them, so kinds beyond long-running turns
-- (shared agents, integrations) can be shown with their sender.
ALTER TABLE notifications
    ADD COLUMN source_type VARCHAR(50),
    ADD COLUMN source_id TEXT,
    ADD COLUMN source_name TEXT;

COMMENT ON COLUMN notifications.source_type IS
    'What sent the notification: agent or system. NULL for rows written before sources existed.';

-- Long-running turn notifications now fire only for the recipient's own
-- chats and open the chat. Drop the ones raised for other sessions and point
-- the rest at the chat view.
DELETE FROM notifications n
WHERE n.kind = 'turn.long_running_completed'
  AND NOT EXISTS (
      SELECT 1
      FROM sessions s
      WHERE n.target_type = 'session'
        AND s.id = CASE
            WHEN n.target_id ~ '^session_[0-9a-f]{32}$'
            THEN substring(n.target_id FROM 9)::uuid
        END
        AND s.source = 'chat'
        AND s.resolved_owner_user_id = n.user_id
  );

UPDATE notifications
SET href = '/chats/' || target_id
WHERE kind = 'turn.long_running_completed'
  AND target_type = 'session';
