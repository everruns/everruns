-- Harness icon: display glyph name rendered by the UI.
--
-- Built-in harness definitions declare their icon in code and it is synced on
-- org provisioning. Custom harnesses leave it NULL and fall back to the
-- generic harness glyph in the UI.
ALTER TABLE harnesses ADD COLUMN icon VARCHAR(64);
