-- EVE-810: host-managed provider rows.
--
-- A provider marked `managed` is owned by the host/embedder (e.g. a built-in
-- LLM gateway provisioned per org). The OSS providers API refuses PATCH/DELETE
-- on such a row (returns 403) so tenant admins cannot edit its credentials,
-- repoint its base_url, or delete it. Model sync stays allowed. Defaults false,
-- so every existing and OSS-created provider is tenant-writable as before.
ALTER TABLE providers ADD COLUMN managed BOOLEAN NOT NULL DEFAULT FALSE;
