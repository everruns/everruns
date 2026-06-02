# SaaS Architecture

## Abstract

This spec captures constraints and postures specific to multi-tenant SaaS deployments of Everruns. Self-hosted (OSS) and SaaS share the same codebase; the differences lie in deployment configuration and operational decisions that must be made intentionally at each boundary.

## BYOK-Only Posture

Everruns is **Bring-Your-Own-Key (BYOK)** for LLM provider credentials. Every tenant that wants to execute LLM-backed agents must configure their own provider credentials through the Everruns UI (Settings → LLM Providers) or API. Platform operator credentials must never be exposed to the tenant execution path.

### What This Means in Practice

1. **No raw provider keys in the server/worker process environment.** Do not set `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GEMINI_API_KEY`, `OPENROUTER_API_KEY`, or any similar raw provider key as an environment variable on the request-serving server or worker processes in a multi-tenant deployment.

2. **No `DEFAULT_*` env vars in production SaaS.** The `DEFAULT_OPENAI_API_KEY`, `DEFAULT_ANTHROPIC_API_KEY`, `DEFAULT_GEMINI_API_KEY`, and `DEFAULT_AZURE_OPENAI_API_KEY` env vars are not consulted by the current key resolver (removed in EVE-511/512). They must not be reintroduced: setting them on a multi-tenant server process would create a cost-runaway path where agents for orgs without configured keys spend platform credentials instead of failing with a clear auth error.

3. **Key resolution is fail-closed.** `LlmResolverService::resolve_provider_api_key` and `resolve_provider_credentials` return `None` when no encrypted key exists in the database. They never fall back to environment variables. See `crates/server/src/services/llm_resolver.rs` (TM-LLM-022).

4. **New orgs start with no providers.** Default seeds create provider *configurations* (name, type, base URL) but no API keys. Agents for a new org fail with a clear auth error rather than silently spending platform credentials.

### Utility LLM

`UTILITY_OPENAI_API_KEY` is a separate platform-internal key used exclusively for platform features such as tool search and embeddings in the context of platform operations. It is not a fallback for tenant agent execution and must be scoped to a dedicated, non-tenant-serving process or service account.

### Model Sync

The background model-sync job (`ModelSyncService`) discovers available models from provider APIs. When a provider has no encrypted key in the database and no encryption service is available, sync is skipped for that provider (fail-closed). Model sync that requires dedicated platform credentials (for a platform-managed default provider catalog) must use a service account separate from the request-serving server process.

## Pre-Signup Checklist

Before enabling open signup on a production SaaS deployment, verify:

- [ ] No `DEFAULT_OPENAI_API_KEY` in the server/worker env.
- [ ] No `DEFAULT_ANTHROPIC_API_KEY` in the server/worker env.
- [ ] No `DEFAULT_GEMINI_API_KEY` in the server/worker env.
- [ ] No `DEFAULT_AZURE_OPENAI_API_KEY` in the server/worker env.
- [ ] No `UTILITY_OPENAI_API_KEY` accessible to the tenant execution path.
- [ ] No raw `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `GEMINI_API_KEY`, or `OPENROUTER_API_KEY` in the server/worker env.
- [ ] `E2B_API_KEY`, `DENO_DEPLOY_TOKEN`, and equivalent execution-harness keys are scoped to the execution harness process, not the control-plane server.
- [ ] `SECRETS_ENCRYPTION_KEY` is set and backed by a KMS-managed key (not a manually generated secret).

## References

- `specs/threat-model.md` — TM-LLM-022 (env fallback removal), TM-TENANT-008 (org-scoped user list)
- `specs/models.md` — LLM provider key resolution
- `specs/encryption.md` — envelope encryption for provider keys
- `specs/llm-drivers.md` — key resolution contract (fail-closed)
- `crates/server/src/services/llm_resolver.rs` — resolver implementation
