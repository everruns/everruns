-- Sidecar tables for the optional S3-compatible blob backend
-- (specs/object-storage.md).
--
-- When STORAGE_BLOB_BACKEND=s3, file/image *bytes* live in the object store and
-- these tables hold the object pointer plus a content hash for compare-and-set.
-- The main workspace_files.content / images.data columns are left empty for
-- offloaded rows. The default (db) deployment never writes these tables, so
-- this migration is purely additive and changes no existing behavior.
--
-- Keying by the owning row id with ON DELETE CASCADE means metadata deletes
-- clean up the sidecar pointer automatically. The blob object itself is removed
-- explicitly by the repository (object stores are not transactional with
-- PostgreSQL); orphaned objects from interrupted deletes are reclaimed by a
-- separate GC pass (see spec).

CREATE TABLE workspace_file_blobs (
    file_id UUID PRIMARY KEY REFERENCES workspace_files(id) ON DELETE CASCADE,
    -- Opaque object-store key (relative to the deployment prefix).
    blob_key TEXT NOT NULL,
    -- Hex SHA-256 of the offloaded content; backs edit_file compare-and-set
    -- without round-tripping the bytes through PostgreSQL.
    content_sha256 TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE image_blobs (
    image_id UUID PRIMARY KEY REFERENCES images(id) ON DELETE CASCADE,
    data_key TEXT NOT NULL,
    thumbnail_key TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
