-- Session SQL Databases
-- Page-level SQLite storage backed by PostgreSQL
--
-- Each session can have multiple named SQLite databases.
-- Database content is stored as individual 4KB pages for efficient
-- partial read/write via custom SQLite VFS.

CREATE TABLE session_databases (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    size_bytes BIGINT NOT NULL DEFAULT 0,
    page_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT session_databases_unique_name UNIQUE (session_id, name),
    CONSTRAINT session_databases_name_check CHECK (name ~ '^[a-zA-Z_][a-zA-Z0-9_]{0,63}$')
);

CREATE INDEX idx_session_databases_session_id ON session_databases(session_id);

CREATE TRIGGER update_session_databases_updated_at
    BEFORE UPDATE ON session_databases
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

CREATE TABLE session_database_pages (
    database_id UUID NOT NULL REFERENCES session_databases(id) ON DELETE CASCADE,
    page_number INTEGER NOT NULL,
    data BYTEA NOT NULL,
    PRIMARY KEY (database_id, page_number)
);

CREATE INDEX idx_session_database_pages_db ON session_database_pages(database_id);
