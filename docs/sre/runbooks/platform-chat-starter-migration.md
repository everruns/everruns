---
title: Migration 144 Fails on Duplicate Platform Chat Starters
description: "Clear user-set platform-chat-starter tags that block migration 144's unique index, then restart."
---

Migration `144_platform_chat_starter_unique.sql` can fail on startup with a duplicate key violation, which stops the server before it finishes migrating.

```
ERROR: could not create unique index "idx_sessions_platform_chat_starter_owner"
DETAIL: Key (org_id, owner_principal_id)=(…, …) is duplicated.
```

## Why it happens

Migration 144 makes `platform-chat-starter` the marker for an owner's single automatic Platform Chat, and enforces it with a partial unique index on `(org_id, owner_principal_id)`.

Before 144 that tag was not reserved, and `sessions.tags` is user-editable. Any session could already carry it. The migration elects a canonical starter only among sessions that do **not** already have the tag:

```sql
AND NOT 'platform-chat-starter' = ANY(s.tags);
```

So two pre-existing, user-tagged sessions belonging to one owner are both skipped by the election and then collide on the unique index.

## Why this needs an operator

The fix cannot ship as a migration:

- Migration 144 is immutable once merged. SQLx records each migration's checksum in `_sqlx_migrations`, so editing it breaks startup on every database that already applied the original.
- A later migration cannot help either. SQLx applies in version order and halts on failure, so a database stuck at 144 never reaches 145 or beyond.

A database in this state has to be cleared by hand before the server can start.

## Diagnosis

Find the owners with more than one tagged session:

```sql
SELECT org_id, owner_principal_id, count(*) AS tagged, array_agg(id) AS session_ids
FROM sessions
WHERE 'platform-chat-starter' = ANY(tags)
GROUP BY org_id, owner_principal_id
HAVING count(*) > 1;
```

No rows means this is not the cause and the failure is something else.

## Remediation

Every pre-144 use of the tag is user-supplied and carries no meaning the product assigns to it, so clear all of them and let the migration elect starters itself:

```sql
BEGIN;

-- Confirm the blast radius before changing anything.
SELECT count(*) FROM sessions WHERE 'platform-chat-starter' = ANY(tags);

UPDATE sessions
SET tags = array_remove(tags, 'platform-chat-starter')
WHERE 'platform-chat-starter' = ANY(tags);

COMMIT;
```

Then restart the server. Migration 144 runs from a clean state: it elects one starter per owner, creates the unique index, and the triggers keep the tag authoritative from then on.

## Verification

```sql
-- migration recorded
SELECT version, description, success FROM _sqlx_migrations WHERE version = 144;

-- at most one starter per owner
SELECT org_id, owner_principal_id, count(*)
FROM sessions
WHERE 'platform-chat-starter' = ANY(tags)
GROUP BY org_id, owner_principal_id
HAVING count(*) > 1;
```

The first should report `success = true`; the second should return no rows.

## Notes

Running the `UPDATE` on a database where 144 already succeeded is not useful and is best avoided: the tag is authoritative there, the `keep_platform_chat_starter_tag` trigger restores it on update, and the unique index already guarantees there is nothing to clean.
