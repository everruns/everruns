-- Update default model from gpt-5-mini to gpt-5.2
-- Also removes gpt-5-mini and gpt-5 from favorites

-- Remove default flag from gpt-5-mini
UPDATE llm_models
SET is_default = false,
    is_favorite = false,
    updated_at = NOW()
WHERE id = '01933b5a-0000-7000-8000-00000000020a';

-- Set gpt-5.2 as the new default
UPDATE llm_models
SET is_default = true,
    updated_at = NOW()
WHERE id = '01933b5a-0000-7000-8000-000000000201';

-- Ensure gpt-5 is not a favorite (should already be false, but ensure consistency)
UPDATE llm_models
SET is_favorite = false,
    updated_at = NOW()
WHERE id = '01933b5a-0000-7000-8000-00000000020b'
  AND is_favorite = true;
