-- Revert TM-CRYPTO-007: System prompt encryption removed.
-- Encrypting system prompts doesn't address the root concern; PII should not
-- be in system prompts at all. Drop the unused encrypted columns.

ALTER TABLE agents DROP COLUMN IF EXISTS system_prompt_encrypted;
ALTER TABLE harnesses DROP COLUMN IF EXISTS system_prompt_encrypted;
