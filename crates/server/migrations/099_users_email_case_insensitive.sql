-- Enforce case-insensitive email identity (EVE-704).
--
-- Emails were stored and looked up case-sensitively, so `Alice@example.com` and
-- `alice@example.com` were distinct accounts for a single mailbox. Canonicalize
-- existing rows to lowercase and replace the case-sensitive unique index with a
-- functional unique index on lower(email), matching the application-level
-- normalization applied in `StorageBackend` (normalize_email).
--
-- NOTE: if a deployment already has case-variant duplicate accounts for the same
-- mailbox, the UPDATE cannot merge them and the unique index build fails; such
-- duplicates must be reconciled manually before applying this migration.
UPDATE users SET email = lower(email) WHERE email <> lower(email);

DROP INDEX idx_users_email;
CREATE UNIQUE INDEX idx_users_email ON users (lower(email));
