-- A restore is recorded as a change of its own (action `restored`), and says
-- which revision it brought back, so history reads "restored to revision 4"
-- rather than an anonymous update. See
-- knowledge/execution/change-reasons-and-manager-context.md.
ALTER TABLE entity_changes ADD COLUMN restored_from_revision BIGINT NULL;
