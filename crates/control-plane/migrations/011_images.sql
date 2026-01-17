-- Images table for storing uploaded images
-- Global images with optional session_id in metadata for tracking
--
-- Design decisions:
-- - Images stored globally (not session-scoped lifecycle)
-- - session_id in metadata for tracking/analytics only
-- - thumbnail_data generated on upload for efficient display
-- - Supported formats: PNG, JPEG, GIF, WebP (OpenAI Vision compatible)
-- - 100MB hard limit enforced at application layer

CREATE TABLE images (
    id UUID PRIMARY KEY DEFAULT uuidv7(),

    -- Original file information
    filename TEXT NOT NULL,
    content_type TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,

    -- Binary data (original and thumbnail)
    data BYTEA NOT NULL,
    thumbnail_data BYTEA,
    thumbnail_content_type TEXT,

    -- Metadata for tracking (includes session_id if uploaded from a session)
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,

    -- Timestamps
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for listing images by creation date
CREATE INDEX idx_images_created_at ON images(created_at DESC);

-- Index for filtering by session_id in metadata
CREATE INDEX idx_images_session_id ON images((metadata->>'session_id'))
    WHERE metadata->>'session_id' IS NOT NULL;

-- Comments
COMMENT ON TABLE images IS 'Global image storage for message attachments';
COMMENT ON COLUMN images.content_type IS 'MIME type: image/png, image/jpeg, image/gif, image/webp';
COMMENT ON COLUMN images.metadata IS 'JSON metadata including optional session_id for tracking';
COMMENT ON COLUMN images.thumbnail_data IS 'Thumbnail image for efficient display (max 200x200)';
