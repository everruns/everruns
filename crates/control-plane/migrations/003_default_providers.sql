-- Default LLM Providers and Models
--
-- These are the built-in providers available out of the box.
-- API keys are set via environment variables at runtime.
-- Model list based on https://models.dev/api.json

-- OpenAI Provider (well-known UUID for reference)
INSERT INTO llm_providers (id, name, provider_type, status)
VALUES ('01933b5a-0000-7000-8000-000000000001', 'OpenAI', 'openai', 'active')
ON CONFLICT (id) DO NOTHING;

-- Anthropic Provider (well-known UUID for reference)
INSERT INTO llm_providers (id, name, provider_type, status)
VALUES ('01933b5a-0000-7000-8000-000000000002', 'Anthropic', 'anthropic', 'active')
ON CONFLICT (id) DO NOTHING;

-- OpenAI Models (sorted by generation, newest first)
INSERT INTO llm_models (provider_id, model_id, display_name, is_default, status) VALUES
    -- GPT-5 series
    ('01933b5a-0000-7000-8000-000000000001', 'gpt-5-mini', 'GPT-5 mini', TRUE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'gpt-5', 'GPT-5', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'gpt-5-nano', 'GPT-5 nano', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'gpt-5-pro', 'GPT-5 Pro', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'gpt-5.1', 'GPT-5.1', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'gpt-5.2', 'GPT-5.2', FALSE, 'active'),
    -- GPT-4 series
    ('01933b5a-0000-7000-8000-000000000001', 'gpt-4.1', 'GPT-4.1', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'gpt-4.1-mini', 'GPT-4.1 mini', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'gpt-4.1-nano', 'GPT-4.1 nano', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'gpt-4o', 'GPT-4o', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'gpt-4o-mini', 'GPT-4o mini', FALSE, 'active'),
    -- Reasoning models (o-series)
    ('01933b5a-0000-7000-8000-000000000001', 'o4-mini', 'o4 mini', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'o3', 'o3', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'o3-mini', 'o3 mini', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'o3-pro', 'o3 Pro', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'o1', 'o1', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'o1-mini', 'o1 mini', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'o1-pro', 'o1 Pro', FALSE, 'active')
ON CONFLICT DO NOTHING;

-- Anthropic Models (sorted by generation, newest first)
INSERT INTO llm_models (provider_id, model_id, display_name, is_default, status) VALUES
    -- Claude 4.5 series
    ('01933b5a-0000-7000-8000-000000000002', 'claude-opus-4-5-20251101', 'Claude Opus 4.5', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000002', 'claude-sonnet-4-5-20250929', 'Claude Sonnet 4.5', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000002', 'claude-haiku-4-5-20251001', 'Claude Haiku 4.5', FALSE, 'active'),
    -- Claude 4 series
    ('01933b5a-0000-7000-8000-000000000002', 'claude-opus-4-20250514', 'Claude Opus 4', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000002', 'claude-sonnet-4-20250514', 'Claude Sonnet 4', FALSE, 'active'),
    -- Claude 3.7
    ('01933b5a-0000-7000-8000-000000000002', 'claude-3-7-sonnet-20250219', 'Claude 3.7 Sonnet', FALSE, 'active'),
    -- Claude 3.5
    ('01933b5a-0000-7000-8000-000000000002', 'claude-3-5-sonnet-20241022', 'Claude 3.5 Sonnet', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000002', 'claude-3-5-haiku-20241022', 'Claude 3.5 Haiku', FALSE, 'active')
ON CONFLICT DO NOTHING;
