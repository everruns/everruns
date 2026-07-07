-- User preferences: a per-user key/value store for client/UI settings.
--
-- Mirrors `session_key_values` but scoped to a user instead of a session, so
-- preferences (e.g. dismissed banners) persist per-user across devices and
-- browsers. Values are stored as TEXT; callers may serialize JSON into them so
-- the schema stays agnostic about the shape of any individual preference.
CREATE TABLE user_preferences (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,

    -- Key name (unique per user)
    key VARCHAR(255) NOT NULL,

    -- Value (stored as text, can be JSON)
    value TEXT NOT NULL,

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- One value per key per user
    CONSTRAINT user_preferences_unique_key UNIQUE (user_id, key)
);

CREATE INDEX idx_user_preferences_user_id ON user_preferences(user_id);

CREATE TRIGGER update_user_preferences_updated_at
    BEFORE UPDATE ON user_preferences
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();
