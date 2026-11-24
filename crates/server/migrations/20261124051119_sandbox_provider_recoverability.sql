-- sandbox_provider_recoverability
-- Recovery availability belongs to the same fenced provider observation as
-- state/external ID. NULL preserves uncertainty until a successful refresh.
ALTER TABLE sandboxes ADD COLUMN provider_status_recoverable BOOLEAN;
