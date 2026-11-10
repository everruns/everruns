-- A room agent member may wake on relevant messages: a message nobody
-- addressed to it is checked by a decision model (or, without one, the
-- agent's own model), and it answers when the check says the message is for
-- it (knowledge/runtime-resources/rooms.md, "Waking agents").
ALTER TABLE room_members DROP CONSTRAINT room_members_wake_check;
ALTER TABLE room_members ADD CONSTRAINT room_members_wake_check
    CHECK (wake IN ('mentions', 'everything', 'relevant', 'never'));
