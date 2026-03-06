-- TM-CRYPTO-007: Extend encryption scope to system prompts.
-- Add encrypted system_prompt columns alongside existing plaintext columns.
-- Encrypted column populated on next write; reads prefer encrypted when available.

ALTER TABLE agents ADD COLUMN system_prompt_encrypted BYTEA;
ALTER TABLE harnesses ADD COLUMN system_prompt_encrypted BYTEA;
