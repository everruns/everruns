-- A person's Good / Bad rating of one message in a session.
--
-- One row per person and message: rating again replaces the row, clearing
-- deletes it. `message_id` is the public message id (`msg_...`) stored in the
-- message event, so it is text and has no foreign key; the row goes with its
-- session. Session Trace reads these later.
-- See knowledge/ui/chat-experience.md ("Message feedback").

CREATE TABLE message_feedback (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    message_id TEXT NOT NULL,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    rating TEXT NOT NULL CHECK (rating IN ('good', 'bad')),
    comment TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (session_id, message_id, user_id)
);

CREATE INDEX idx_message_feedback_org_session ON message_feedback(org_id, session_id);

CREATE TRIGGER update_message_feedback_updated_at BEFORE UPDATE ON message_feedback
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
