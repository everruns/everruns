-- Default LLM Providers and Models
--
-- These are the built-in providers available out of the box.
-- API keys are set via environment variables at runtime.

-- OpenAI Provider (well-known UUID for reference)
INSERT INTO llm_providers (id, name, provider_type, status)
VALUES ('01933b5a-0000-7000-8000-000000000001', 'OpenAI', 'openai', 'active')
ON CONFLICT (id) DO NOTHING;

-- Anthropic Provider (well-known UUID for reference)
INSERT INTO llm_providers (id, name, provider_type, status)
VALUES ('01933b5a-0000-7000-8000-000000000002', 'Anthropic', 'anthropic', 'active')
ON CONFLICT (id) DO NOTHING;

-- OpenAI Models
INSERT INTO llm_models (provider_id, model_id, display_name, is_default, status) VALUES
    ('01933b5a-0000-7000-8000-000000000001', 'gpt-4.1', 'GPT-4.1', TRUE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'gpt-4.1-mini', 'GPT-4.1 mini', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'gpt-4.1-nano', 'GPT-4.1 nano', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'gpt-4o', 'GPT-4o', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'gpt-4o-mini', 'GPT-4o mini', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'o3', 'o3', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'o3-mini', 'o3 mini', FALSE, 'active'),
    ('01933b5a-0000-7000-8000-000000000001', 'o4-mini', 'o4 mini', FALSE, 'active')
ON CONFLICT DO NOTHING;

-- Anthropic Models
INSERT INTO llm_models (provider_id, model_id, display_name, is_default, status) VALUES
    ('01933b5a-0000-7000-8000-000000000002', 'claude-sonnet-4-20250514', 'Claude Sonnet 4', TRUE, 'active'),
    ('01933b5a-0000-7000-8000-000000000002', 'claude-3-5-haiku-20241022', 'Claude 3.5 Haiku', FALSE, 'active')
ON CONFLICT DO NOTHING;
