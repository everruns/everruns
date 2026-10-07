// Service seeding module for server (control plane)
// Decision: Always seed on startup (no flag needed)
// Decision: Prepare auth identities before serving; seed catalogs in background
// Decision: Auth readiness errors stop startup; catalog errors are non-fatal
// Decision: Use fixed UUIDs for idempotency
// Decision: Upsert seed data on conflict, only when values changed (ON CONFLICT DO UPDATE ... WHERE differs)
// Decision: Modular design allows easy addition of new seeders

use crate::auth::config::{AdminConfig, AuthConfig, AuthMode};
use crate::org_init;
use crate::storage::{
    EncryptionService, StorageBackend,
    models::{
        CreateModelRow, CreateOrganizationRow, CreateProviderRow, CreateUserRow, ModelRow,
        UpdateModel, UpdateProvider,
    },
    password::hash_password,
};
use everruns_core::host::HostComposition;
use everruns_core::{DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, DeploymentGrade};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use uuid::Uuid;
mod anonymous;
mod mcp_servers;
mod models;
use anonymous::seed_auth_prerequisites;
use mcp_servers::seed_mcp_servers;
use models::SEED_MODELS;

/// Well-known UUIDs for seed data
/// Format: 01933b5a-0000-7000-8000-0000000001xx
/// Range allocation:
///   - 0x001-0x0FF: LLM Providers
///   - 0x100-0x1FF: Agents
///   - 0x200-0x2FF: OpenAI Models
///   - 0x300-0x3FF: Anthropic Models
///   - 0x400-0x4FF: LlmSim Models
mod seed_ids {
    use uuid::Uuid;

    // LLM Providers (0x001-0x0FF)
    pub const OPENAI_PROVIDER: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000001);
    pub const ANTHROPIC_PROVIDER: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000002);
    pub const LLMSIM_PROVIDER: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000003);
    pub const GEMINI_PROVIDER: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000004);
    pub const BEDROCK_PROVIDER: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000005);
    pub const OPENROUTER_PROVIDER: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000006);
    pub const MAI_PROVIDER: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000007);
    pub const FIREWORKS_PROVIDER: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000008);
    pub const META_PROVIDER: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000009);

    // Agents (0x100-0x1FF)
    pub const DAD_JOKES_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000101);
    pub const RESEARCH_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000102);
    pub const MS_LEARN_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000103);
    pub const PYTHON_CODER_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000104);
    pub const SHELL_ASSISTANT_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000105);
    pub const DATA_ANALYST_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000106);
    pub const E2B_CODER_AGENT: Uuid = Uuid::from_u128(0x0195bb5a_0000_7000_8000_00000000010f);
    pub const DENO_CODER_AGENT: Uuid = Uuid::from_u128(0x0195bb5a_0000_7000_8000_000000000110);
    pub const SPRITES_CODER_AGENT: Uuid = Uuid::from_u128(0x0195bb5a_0000_7000_8000_000000000111);
    // 0x…0109: retired Cloud Cost & Security Auditor demo agent (EVE-875). Do not reuse. 0x…0118: Modal Coder, in seed/agents.rs (this file may not grow).
    pub const PLATFORM_MANAGER_AGENT: Uuid =
        Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000010a);
    pub const WEB_RESEARCHER_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000010b);
    pub const BROWSER_TESTER_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000010c);
    pub const DASHBOARD_BUILDER_AGENT: Uuid =
        Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000010d);
    pub const TASK_ORCHESTRATOR_AGENT: Uuid =
        Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000010e);
    pub const KNOWLEDGE_BASE_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000112);
    pub const IMAGE_STUDIO_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000114);
    pub const CURSOR_AGENT_MANAGER: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000115);
    pub const GUARDED_BASH_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000116);
    pub const CAPABILITY_SCOUT_AGENT: Uuid =
        Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000117);

    // MCP Servers (0x500-0x5FF)
    pub const MS_LEARN_MCP: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000501);
    pub const LINEAR_MCP: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000502);

    // OpenAI Models (0x200-0x2FF)
    pub const GPT_5_2: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000201);
    pub const GPT_5_2_PRO: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000202);
    pub const GPT_5_2_CODEX: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000203);
    pub const GPT_5_2_CHAT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000204);
    pub const GPT_5_1: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000205);
    pub const GPT_5_1_CODEX: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000206);
    pub const GPT_5_1_CODEX_MINI: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000207);
    pub const GPT_5_1_CODEX_MAX: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000208);
    pub const GPT_5_1_CHAT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000209);
    pub const GPT_5_MINI: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000020a);
    pub const GPT_5: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000020b);
    pub const GPT_5_NANO: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000020c);
    pub const GPT_5_PRO: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000020d);
    pub const GPT_5_CODEX: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000020e);
    pub const GPT_5_CHAT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000020f);
    pub const GPT_4_1: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000210);
    pub const GPT_4_1_MINI: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000211);
    pub const GPT_4_1_NANO: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000212);
    pub const O4_MINI: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000215);
    pub const O4_MINI_DEEP_RESEARCH: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000216);
    pub const O3: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000217);
    pub const O3_PRO: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000219);
    pub const O3_DEEP_RESEARCH: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000021a);
    pub const GPT_5_4: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000021f);
    pub const GPT_5_4_PRO: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000220);
    pub const GPT_5_4_MINI: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000221);
    pub const GPT_5_4_NANO: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000222);
    pub const GPT_5_5: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000223);
    pub const GPT_5_5_PRO: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000224);
    pub const GPT_REALTIME_2: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000225);
    pub const GPT_5_6_SOL: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000226);
    pub const GPT_5_6_TERRA: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000227);
    pub const GPT_5_6_LUNA: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000228);
    pub const TEXT_EMBEDDING_3_SMALL: Uuid =
        Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000229);
    pub const GPT_6_ASTRA: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000022a);
    pub const GPT_6_SOL: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000022b);
    pub const GPT_6_LUNA: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000022c);
    pub const GPT_6_1_SOL: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000022d);

    // Anthropic Models (0x300-0x3FF)
    pub const CLAUDE_FABLE_5_1: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000030f);
    pub const CLAUDE_OPUS_5_5: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000310);
    pub const CLAUDE_OPUS_5: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000030e);
    pub const CLAUDE_SONNET_5_5: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000311);
    pub const CLAUDE_SONNET_5: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000030c);
    pub const CLAUDE_HAIKU_5_5: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000312);
    pub const CLAUDE_OPUS_4_8: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000030d);
    pub const CLAUDE_OPUS_4_7: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000309);
    pub const CLAUDE_SONNET_4_6: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000030a);
    pub const CLAUDE_HAIKU_4_6: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000030b);
    pub const CLAUDE_OPUS_4_5: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000301);
    // 0x…0302: retired Claude Sonnet 4.5 (sunset by Anthropic). Do not reuse.
    pub const CLAUDE_HAIKU_4_5: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000303);
    pub const CLAUDE_OPUS_4: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000304);
    // 1M-context variants (`[1m]` model ids), 0x3a0+ sub-range.
    pub const CLAUDE_FABLE_5_1_1M: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_0000000003a9);
    pub const CLAUDE_OPUS_5_5_1M: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_0000000003aa);
    pub const CLAUDE_OPUS_5_1M: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_0000000003a8);
    pub const CLAUDE_OPUS_4_7_1M: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_0000000003a7);
    pub const CLAUDE_SONNET_5_5_1M: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_0000000003ab);
    pub const CLAUDE_SONNET_5_1M: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_0000000003a5);
    pub const CLAUDE_HAIKU_5_5_1M: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_0000000003ac);

    // LlmSim Models (0x400-0x4FF)
    pub const LLMSIM_DEFAULT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000401);
    pub const LLMSIM_LATENCY: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000402);

    // Gemini Models (0x600-0x6FF)
    pub const GEMINI_31_PRO_PREVIEW: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000604);
    pub const GEMINI_35_FLASH: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000605);
    pub const GEMINI_31_FLASH_LITE: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000606);
    pub const GEMINI_25_PRO: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000601);
    pub const GEMINI_25_FLASH: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000602);
    pub const GEMINI_20_FLASH: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000603);

    // Bedrock Models (0x700-0x7FF)
    pub const BEDROCK_CLAUDE_OPUS_5_5: Uuid =
        Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000705);
    pub const BEDROCK_CLAUDE_OPUS_5: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000703);
    pub const BEDROCK_CLAUDE_SONNET_5_5: Uuid =
        Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000706);
    pub const BEDROCK_CLAUDE_SONNET_5: Uuid =
        Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000704);
}

// ---- Seeder Result ----

/// Result of running a seeder
#[derive(Debug, Default)]
pub struct SeedResult {
    created: usize,
    updated: usize,
    unchanged: usize,
}

impl SeedResult {
    fn merge(&mut self, other: SeedResult) {
        self.created += other.created;
        self.updated += other.updated;
        self.unchanged += other.unchanged;
    }

    fn has_changes(&self) -> bool {
        self.created > 0 || self.updated > 0
    }
}

// ---- Default Organization Seeder ----

/// Seed the default organization (must run first).
/// Orgs use DO NOTHING since their seed data is static.
async fn seed_default_organization(db: &StorageBackend) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();

    let input = CreateOrganizationRow {
        public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
        name: "Default Organization".to_string(),
        created_by: None,
    };

    match db
        .create_organization_with_id(DEFAULT_ORG_ID, input)
        .await?
    {
        Some(_) => {
            tracing::info!(
                org_id = DEFAULT_ORG_ID,
                public_id = DEFAULT_ORG_PUBLIC_ID,
                "Created default organization"
            );
            // Seed the default plugin marketplace for the freshly created default org.
            org_init::seed_default_plugin_marketplace(db, DEFAULT_ORG_ID).await;
            result.created += 1;
        }
        None => {
            tracing::debug!(
                org_id = DEFAULT_ORG_ID,
                public_id = DEFAULT_ORG_PUBLIC_ID,
                "Default organization up to date"
            );
            result.unchanged += 1;
        }
    }

    Ok(result)
}

// ---- Admin User Seeder ----

/// Seed admin user for auth=admin mode.
/// Creates the admin user at startup so they have an org membership
/// before first login. Uses a well-known UUID for idempotency.
const ADMIN_USER_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_000000000002);

async fn seed_admin_user(
    db: &StorageBackend,
    admin_config: &AdminConfig,
    harness_definitions: &[crate::records::BuiltInHarnessDefinition],
) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();

    if let Some(existing_admin) = db.get_user_by_email(&admin_config.email).await? {
        let existing_roles: Vec<String> =
            serde_json::from_value(existing_admin.roles.clone()).unwrap_or_default();
        let is_admin = existing_roles.iter().any(|role| role == "admin");
        if !is_admin || !existing_admin.email_verified {
            return Err(anyhow::anyhow!(
                "Refusing to seed admin user for existing untrusted account (email={}, user_id={})",
                admin_config.email,
                existing_admin.id
            ));
        }

        tracing::debug!(
            user_id = %existing_admin.id,
            "Admin user already exists by email"
        );
        result.unchanged += 1;

        db.ensure_membership(existing_admin.id, DEFAULT_ORG_ID, "owner")
            .await?;

        org_init::initialize_org_harnesses_with_definitions(
            db,
            DEFAULT_ORG_ID,
            harness_definitions,
        )
        .await?;

        return Ok(result);
    }

    let password_hash = hash_password(&admin_config.password)
        .map_err(|e| anyhow::anyhow!("Failed to hash admin password: {}", e))?;

    let input = CreateUserRow {
        email: admin_config.email.clone(),
        name: "Admin".to_string(),
        avatar_url: None,
        roles: vec!["admin".to_string()],
        password_hash: Some(password_hash),
        email_verified: true,
        auth_provider: Some("local".to_string()),
        auth_provider_id: None,
        external_id: None,
    };

    // Try creating with well-known ID; if user already exists by email,
    // look them up instead (admin may have been created via login before
    // this seeder existed).
    match db.create_user_with_id(ADMIN_USER_ID, input).await? {
        Some(_) => {
            tracing::info!("Created admin user");
            result.created += 1;
        }
        None => {
            tracing::debug!("Admin user up to date");
            result.unchanged += 1;
        }
    }

    // Resolve the actual user id — may differ from ADMIN_USER_ID if the
    // admin was created via login (which assigns a random UUID).
    let user_id = db
        .get_user_by_email(&admin_config.email)
        .await?
        .map(|u| u.id)
        .unwrap_or(ADMIN_USER_ID);

    // Ensure admin user is owner of default org
    db.ensure_membership(user_id, DEFAULT_ORG_ID, "owner")
        .await?;

    // Ensure default org has built-in harnesses (same safety net as registration handlers)
    org_init::initialize_org_harnesses_with_definitions(db, DEFAULT_ORG_ID, harness_definitions)
        .await?;

    Ok(result)
}

// ============================================
// Agent Seeder
// ============================================

/// Capability entry with optional per-capability config.
pub(crate) struct SeedCapability {
    pub(crate) id: &'static str,
    pub(crate) config: Option<fn() -> serde_json::Value>,
}

impl SeedCapability {
    pub(crate) const fn new(id: &'static str) -> Self {
        Self { id, config: None }
    }

    pub(crate) const fn with_config(id: &'static str, config: fn() -> serde_json::Value) -> Self {
        Self {
            id,
            config: Some(config),
        }
    }
}

impl std::fmt::Display for SeedCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id)
    }
}
/// Seed agent definition (used by agent examples API, not auto-seeded into DB)
pub(crate) struct SeedAgent {
    /// Retained for stable identification; not auto-seeded into DB.
    #[allow(dead_code)]
    pub(crate) id: Uuid,
    /// Unique name (e.g. "dad-jokes-agent").
    pub(crate) name: &'static str,
    /// Human-readable display name (e.g. "Dad Jokes Agent").
    pub(crate) display_name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) system_prompt: &'static str,
    pub(crate) harness_name: &'static str,
    pub(crate) tags: &'static [&'static str],
    pub(crate) capabilities: &'static [SeedCapability],
    /// If true, only seed in dev environments (experimental features)
    pub(crate) dev_only: bool,
}

mod agents;
pub(crate) use agents::SEED_AGENTS;

// Agents are NOT auto-seeded. They live as examples (agent_examples.rs) and are adopted on
// demand via POST /v1/agent-examples/{slug}/use. This prevents duplicate agents.

// ============================================
// LLM Provider Seeder
// ============================================

/// Seed LLM provider definition
struct SeedProvider {
    id: Uuid,
    name: &'static str,
    provider_type: &'static str,
}

/// Built-in seed providers
const SEED_PROVIDERS: &[SeedProvider] = &[
    SeedProvider {
        id: seed_ids::OPENAI_PROVIDER,
        name: "OpenAI",
        provider_type: "openai",
    },
    SeedProvider {
        id: seed_ids::ANTHROPIC_PROVIDER,
        name: "Anthropic",
        provider_type: "anthropic",
    },
    SeedProvider {
        id: seed_ids::OPENROUTER_PROVIDER,
        name: "OpenRouter",
        provider_type: "openrouter",
    },
    SeedProvider {
        id: seed_ids::GEMINI_PROVIDER,
        name: "Google Gemini",
        provider_type: "gemini",
    },
    SeedProvider {
        id: seed_ids::LLMSIM_PROVIDER,
        name: "LlmSim",
        provider_type: "llmsim",
    },
    SeedProvider {
        id: seed_ids::BEDROCK_PROVIDER,
        name: "AWS Bedrock",
        provider_type: "bedrock",
    },
    SeedProvider {
        id: seed_ids::MAI_PROVIDER,
        name: "Microsoft MAI",
        provider_type: "mai",
    },
    SeedProvider {
        id: seed_ids::FIREWORKS_PROVIDER,
        name: "Fireworks AI",
        provider_type: "fireworks",
    },
    SeedProvider {
        id: seed_ids::META_PROVIDER,
        name: "Meta Model API",
        provider_type: "meta",
    },
];

fn enabled_seed_provider_ids(host_composition: &HostComposition) -> HashSet<Uuid> {
    let registered_provider_types: HashSet<String> = host_composition
        .driver_registry()
        .registered_providers()
        .into_iter()
        .map(|provider_type| provider_type.to_string())
        .collect();

    SEED_PROVIDERS
        .iter()
        .filter(|seed| registered_provider_types.contains(seed.provider_type))
        .map(|seed| seed.id)
        .collect()
}

async fn seed_providers_with_host_composition(
    db: &StorageBackend,
    host_composition: &HostComposition,
) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();
    let enabled_provider_ids = enabled_seed_provider_ids(host_composition);

    for seed in SEED_PROVIDERS {
        if !enabled_provider_ids.contains(&seed.id) {
            tracing::debug!(
                name = seed.name,
                id = %seed.id,
                provider_type = seed.provider_type,
                "Skipping seed provider because platform definition does not register its driver"
            );
            continue;
        }
        let input = CreateProviderRow {
            name: seed.name.to_string(),
            provider_type: seed.provider_type.to_string(),
            base_url: None,
            api_key_encrypted: None, // No secret in seed data
            settings: None,
        };

        match db
            .create_provider_with_id(DEFAULT_ORG_ID, seed.id, input)
            .await?
        {
            Some(row) => {
                if row.created_at == row.updated_at {
                    tracing::info!(name = seed.name, id = %seed.id, "Created seed provider");
                    result.created += 1;
                } else {
                    tracing::info!(name = seed.name, id = %seed.id, "Updated seed provider");
                    result.updated += 1;
                }
            }
            None => {
                tracing::debug!(name = seed.name, id = %seed.id, "Provider up to date");
                result.unchanged += 1;
            }
        }
    }

    Ok(result)
}

/// Whether to materialize `DEFAULT_*_API_KEY` env vars into the default org's
/// provider rows on startup.
///
/// This is a single-tenant / dev convenience. The tenant resolver is
/// fail-closed (EVE-511): it only reads keys from the database and never falls
/// back to process env during turn execution. Materializing the env keys into
/// the default org's seed providers lets a single-org self-hosted deploy or
/// `just start-dev` keep using `DEFAULT_*_API_KEY` without re-opening an
/// implicit env fallback in the hot path.
///
/// Gated so platform-level keys are never seeded into an org that untrusted
/// users can join and spend (EVE-512):
/// - `SEED_DEFAULT_PROVIDER_KEYS_FROM_ENV` unset  -> defaults to `grade.is_dev()`
/// - `true`/`1`/`yes` -> enabled only when dev, or when built-in auth cannot
///   self-provision users into `DEFAULT_ORG_ID` (e.g. full auth with signup
///   disabled and no built-in OAuth, admin-only auth, or external auth)
/// - anything else    -> disabled
fn materialize_env_provider_keys_allowed(grade: DeploymentGrade) -> bool {
    let auth_config = AuthConfig::from_env();
    materialize_env_provider_keys_allowed_with(grade, &auth_config, |name| std::env::var(name).ok())
}

/// Testable core of [`materialize_env_provider_keys_allowed`] with injectable
/// env and auth config. Dev keeps its convenience default; non-dev explicit
/// opt-in is only honored when built-in auth cannot self-provision users into
/// `DEFAULT_ORG_ID`, because members can run sessions that spend these keys.
fn materialize_env_provider_keys_allowed_with<F>(
    grade: DeploymentGrade,
    auth_config: &AuthConfig,
    env_lookup: F,
) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    let enabled = materialize_env_provider_keys_enabled_with(grade, env_lookup);
    if !enabled {
        return false;
    }

    grade.is_dev() || !built_in_default_org_self_provisioning_enabled(auth_config)
}

fn built_in_default_org_self_provisioning_enabled(auth_config: &AuthConfig) -> bool {
    auth_config.signup_enabled() || auth_config.oauth_enabled()
}

/// Env/grade opt-in logic for env-key materialization (injectable env lookup).
/// Does not enforce the auth self-provisioning guard; use
/// [`materialize_env_provider_keys_allowed_with`] for the full decision.
fn materialize_env_provider_keys_enabled_with<F>(grade: DeploymentGrade, env_lookup: F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    match env_lookup("SEED_DEFAULT_PROVIDER_KEYS_FROM_ENV") {
        Some(v) => matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes"),
        None => grade.is_dev(),
    }
}

/// Copy `DEFAULT_<PROVIDER>_API_KEY` env vars into the default org's seed
/// provider rows (encrypted), so org-scoped execution can resolve them through
/// the normal fail-closed DB path.
///
/// Only fills providers that have no key configured — an explicitly-set key
/// (via UI/API) is never overwritten. Idempotent: a provider that has a
/// matching `DEFAULT_*_API_KEY` but already has a key set is left untouched and
/// counted as unchanged. Providers without a matching env var are skipped and
/// not counted at all.
///
/// Callers MUST gate this behind [`materialize_env_provider_keys_allowed`]; it
/// must not run when built-in auth can self-provision users into `DEFAULT_ORG_ID`.
async fn seed_default_provider_keys_from_env(
    db: &StorageBackend,
    encryption: &EncryptionService,
    host_composition: &HostComposition,
) -> anyhow::Result<SeedResult> {
    seed_default_provider_keys_with_lookup(db, encryption, host_composition, |provider_type| {
        crate::services::provider_resolver::get_default_api_key_from_env(provider_type)
    })
    .await
}

/// Testable core of [`seed_default_provider_keys_from_env`] with an injectable
/// per-provider key lookup.
async fn seed_default_provider_keys_with_lookup<F>(
    db: &StorageBackend,
    encryption: &EncryptionService,
    host_composition: &HostComposition,
    key_lookup: F,
) -> anyhow::Result<SeedResult>
where
    F: Fn(&str) -> Option<String>,
{
    let mut result = SeedResult::default();
    let enabled_provider_ids = enabled_seed_provider_ids(host_composition);

    for seed in SEED_PROVIDERS {
        if !enabled_provider_ids.contains(&seed.id) {
            continue;
        }

        let Some(env_key) = key_lookup(seed.provider_type) else {
            continue;
        };

        let Some(provider) = db.get_provider(DEFAULT_ORG_ID, seed.id).await? else {
            // Provider row not seeded (driver not registered for this grade).
            continue;
        };

        // Never overwrite an explicitly-configured key — only fill empty slots.
        if provider.api_key_encrypted.is_some() {
            result.unchanged += 1;
            continue;
        }

        let encrypted = encryption.encrypt_string(&env_key)?;
        db.update_provider(
            DEFAULT_ORG_ID,
            seed.id,
            UpdateProvider {
                name: None,
                provider_type: None,
                base_url: None,
                api_key_encrypted: Some(encrypted),
                status: None,
                settings: None,
            },
        )
        .await?;
        result.updated += 1;
        tracing::info!(
            provider = seed.name,
            "Materialized DEFAULT_*_API_KEY into default-org provider (single-tenant/dev)"
        );
    }

    Ok(result)
}

// ============================================
// LLM Model Seeder
// ============================================

// The model catalogue lives in `seed/models.rs`.

/// Seed LLM models into the database (upserts, only when changed)
async fn seed_models_with_host_composition(
    db: &StorageBackend,
    host_composition: &HostComposition,
) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();
    let enabled_provider_ids = enabled_seed_provider_ids(host_composition);

    // Provider discovery may have catalogued a seed model under a generated id
    // before its seed entry existed. `models(provider_id, model_id)` is unique,
    // so inserting the seed id would fail on every boot (Sentry EVERRUNS-6).
    // Adopt the existing row instead of forcing the seed id onto it.
    let mut existing_by_provider: HashMap<Uuid, Vec<ModelRow>> = HashMap::new();

    for seed in SEED_MODELS {
        if !enabled_provider_ids.contains(&seed.provider_id) {
            tracing::debug!(
                model_id = seed.model_id,
                id = %seed.id,
                "Skipping seed model because its provider driver is not registered by the platform definition"
            );
            continue;
        }
        let existing = match existing_by_provider.entry(seed.provider_id) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => entry.insert(
                db.list_models_for_provider(DEFAULT_ORG_ID, seed.provider_id)
                    .await?,
            ),
        };
        if let Some(row) = existing
            .iter()
            .find(|row| row.model_id == seed.model_id && row.id.uuid() != seed.id)
        {
            let unchanged = row.display_name == seed.display_name
                && row.is_favorite == seed.is_favorite
                && row.enabled == seed.enabled;
            if unchanged {
                tracing::debug!(
                    model_id = seed.model_id,
                    id = %row.id,
                    "Seed model already catalogued by discovery; up to date"
                );
                result.unchanged += 1;
            } else {
                db.update_model(
                    DEFAULT_ORG_ID,
                    row.id.uuid(),
                    UpdateModel {
                        display_name: Some(seed.display_name.to_string()),
                        is_favorite: Some(seed.is_favorite),
                        enabled: Some(seed.enabled),
                        ..UpdateModel::default()
                    },
                )
                .await?;
                tracing::info!(
                    model_id = seed.model_id,
                    id = %row.id,
                    seed_id = %seed.id,
                    "Updated seed model on the row discovery catalogued"
                );
                result.updated += 1;
            }
            continue;
        }
        let input = CreateModelRow {
            provider_id: seed.provider_id.into(),
            model_id: seed.model_id.to_string(),
            display_name: seed.display_name.to_string(),
            capabilities: if seed.model_id.starts_with("text-embedding-") {
                vec!["embeddings".to_string()]
            } else {
                vec![]
            },
            enabled: seed.enabled,
            is_favorite: seed.is_favorite,
            source: "predefined".to_string(), // Seed models are predefined
            provider_metadata: None,
        };

        match db
            .create_model_with_id(DEFAULT_ORG_ID, seed.id, input)
            .await?
        {
            Some(row) => {
                if row.created_at == row.updated_at {
                    tracing::debug!(model_id = seed.model_id, id = %seed.id, "Created seed model");
                    result.created += 1;
                } else {
                    tracing::info!(model_id = seed.model_id, id = %seed.id, "Updated seed model");
                    result.updated += 1;
                }
            }
            None => {
                tracing::debug!(model_id = seed.model_id, id = %seed.id, "Model up to date");
                result.unchanged += 1;
            }
        }
    }

    Ok(result)
}

// ============================================
// Seeding Orchestration
// ============================================

/// Maximum number of retries for transient errors
const MAX_RETRIES: u32 = 5;

/// Initial retry delay (increases exponentially)
const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(2);

/// Optional auth-mode context passed into the seed task so it can
/// pre-create the admin user when `AUTH_MODE=admin`.
#[derive(Clone, Default)]
pub struct SeedAuthContext {
    pub mode: AuthMode,
    pub admin: Option<AdminConfig>,
}

/// Prepare binding upgrades and auth identities before background catalog seeding.
pub async fn prepare_seed_task(
    db: Arc<StorageBackend>,
    auth: &AuthConfig,
    host_composition: HostComposition,
    harnesses: Vec<crate::records::BuiltInHarnessDefinition>,
    encryption: Option<Arc<EncryptionService>>,
) -> anyhow::Result<JoinHandle<()>> {
    // The old default must be pinned before creation traffic can observe the new one.
    if harnesses.iter().any(|h| {
        h.name == "conversation" && h.has_role(crate::records::BuiltInHarnessRole::Default)
    }) {
        org_init::reconcile_built_in_harnesses_with_definitions(&db, &harnesses).await?;
    }
    let auth_ctx = SeedAuthContext {
        mode: auth.mode.clone(),
        admin: auth.admin.clone(),
    };
    // Request ownership and auth-mode cleanup must be ready before HTTP traffic;
    // the delayed catalog task cannot supply these prerequisites safely.
    seed_auth_prerequisites(&db, &auth_ctx, &harnesses).await?;
    Ok(spawn_seed_task_with_host_composition(
        db,
        auth_ctx,
        host_composition,
        harnesses,
        encryption,
    ))
}

/// Spawn seeding as a background task (non-blocking)
/// This allows the HTTP server to start immediately while seeding runs in background.
/// Seeding failures are non-fatal - logged as warnings but don't crash the server.
///
/// Uses `DeploymentGrade::from_env()` to determine which agents to seed.
pub fn spawn_seed_task(db: Arc<StorageBackend>, auth_ctx: SeedAuthContext) -> JoinHandle<()> {
    spawn_seed_task_with_host_composition(
        db,
        auth_ctx,
        crate::platform::oss_host_composition_for_grade(DeploymentGrade::from_env()),
        crate::platform::oss_built_in_harnesses(),
        None,
    )
}

/// Spawn seeding as a background task using an explicit platform definition.
///
/// When `encryption` is available and env-key materialization is enabled for
/// this deployment grade (when environment provider keys may be materialized), the
/// default org's provider rows are seeded with the `DEFAULT_*_API_KEY` env
/// values so single-tenant/dev execution can resolve them via the fail-closed
/// DB path. Multitenant deployments leave this disabled and never spend
/// platform-level keys.
pub fn spawn_seed_task_with_host_composition(
    db: Arc<StorageBackend>,
    auth_ctx: SeedAuthContext,
    host_composition: HostComposition,
    built_in_harnesses: Vec<crate::records::BuiltInHarnessDefinition>,
    encryption: Option<Arc<EncryptionService>>,
) -> JoinHandle<()> {
    let grade = DeploymentGrade::from_env();
    tracing::info!(deployment_grade = %grade, "Starting seeding task");

    tokio::spawn(async move {
        // Small delay to let the server start first
        tokio::time::sleep(Duration::from_millis(500)).await;

        match run_seed_with_retry(
            &db,
            grade,
            &auth_ctx,
            &host_composition,
            &built_in_harnesses,
        )
        .await
        {
            Ok(result) => {
                if result.has_changes() {
                    tracing::info!(
                        created = result.created,
                        updated = result.updated,
                        unchanged = result.unchanged,
                        deployment_grade = %grade,
                        "Seeding complete"
                    );
                } else {
                    tracing::debug!(
                        unchanged = result.unchanged,
                        deployment_grade = %grade,
                        "Seeding complete (all items up to date)"
                    );
                }

                // Single-tenant / dev convenience: materialize DEFAULT_*_API_KEY
                // env vars into the default org's providers. Runs after seed so
                // the provider rows exist; gated so untrusted users cannot join
                // DEFAULT_ORG_ID and spend these keys (EVE-511/EVE-512).
                if materialize_env_provider_keys_allowed(grade) {
                    match &encryption {
                        Some(encryption) => {
                            match seed_default_provider_keys_from_env(
                                &db,
                                encryption,
                                &host_composition,
                            )
                            .await
                            {
                                Ok(r) if r.updated > 0 => tracing::info!(
                                    updated = r.updated,
                                    "Materialized DEFAULT_*_API_KEY into default-org providers"
                                ),
                                Ok(_) => {}
                                Err(e) => tracing::warn!(
                                    error = %e,
                                    "Failed to materialize DEFAULT_*_API_KEY env keys (non-fatal)"
                                ),
                            }
                        }
                        None => tracing::warn!(
                            "DEFAULT_*_API_KEY materialization enabled but encryption is not \
                             configured (set SECRETS_ENCRYPTION_KEY); skipping"
                        ),
                    }
                }

                // After seed succeeds, reconcile built-in harnesses for ALL orgs.
                // Runs inside the seed task (not a separate task) so the default
                // org row is guaranteed to exist before harness init runs.
                reconcile_org_harnesses(&db, &built_in_harnesses).await;
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Seeding failed (non-fatal). Seed data may not be available."
                );
            }
        }
    })
}

/// Reconcile built-in harnesses for every organization (including the default org).
///
/// Called after seeding completes (inside the seed task) so the default org row
/// is guaranteed to exist. Each org is reconciled independently; a single failure
/// is logged but does not prevent other orgs from updating.
async fn reconcile_org_harnesses(
    db: &StorageBackend,
    harnesses: &[crate::records::BuiltInHarnessDefinition],
) {
    let orgs = match db.list_organizations().await {
        Ok(orgs) => orgs,
        Err(e) => {
            tracing::warn!(error = %e, "Harness reconciliation: failed to list orgs (non-fatal)");
            return;
        }
    };

    if orgs.is_empty() {
        tracing::debug!("No orgs to reconcile harnesses for");
        return;
    }

    let mut created = 0usize;
    let mut updated = 0usize;
    let mut unchanged = 0usize;
    let mut errors = 0usize;

    for org in &orgs {
        match org_init::initialize_org_harnesses_with_definitions(db, org.org_id, harnesses).await {
            Ok(r) => {
                created += r.created;
                updated += r.updated;
                unchanged += r.unchanged;
            }
            Err(e) => {
                errors += 1;
                tracing::warn!(
                    org_id = org.org_id,
                    error = %e,
                    "Harness reconciliation failed for org (non-fatal)"
                );
            }
        }
    }

    if created > 0 || updated > 0 || errors > 0 {
        tracing::info!(
            org_count = orgs.len(),
            created,
            updated,
            unchanged,
            errors,
            "Background harness reconciliation complete"
        );
    } else {
        tracing::debug!(
            org_count = orgs.len(),
            unchanged,
            "Background harness reconciliation complete (all up to date)"
        );
    }
}

/// Run seeding with retry logic for transient errors
async fn run_seed_with_retry(
    db: &StorageBackend,
    grade: DeploymentGrade,
    auth_ctx: &SeedAuthContext,
    host_composition: &HostComposition,
    built_in_harnesses: &[crate::records::BuiltInHarnessDefinition],
) -> Result<SeedResult, String> {
    let mut retry_count = 0;
    let mut delay = INITIAL_RETRY_DELAY;

    loop {
        match seed_all_with_host_composition(
            db,
            grade,
            auth_ctx,
            host_composition,
            built_in_harnesses,
        )
        .await
        {
            Ok(result) => return Ok(result),
            Err(e) => {
                let error_str = e.to_string();

                // Check if this is a schema error (table doesn't exist)
                // In this case, don't retry - migrations need to run first
                if error_str.contains("does not exist")
                    || error_str.contains("relation")
                    || error_str.contains("no such table")
                {
                    return Err(format!(
                        "Schema not ready (run migrations first): {}",
                        error_str
                    ));
                }

                retry_count += 1;
                if retry_count > MAX_RETRIES {
                    return Err(format!(
                        "Failed after {} retries: {}",
                        MAX_RETRIES, error_str
                    ));
                }

                tracing::debug!(
                    attempt = retry_count,
                    max_retries = MAX_RETRIES,
                    delay_secs = delay.as_secs(),
                    error = %error_str,
                    "Seeding retry..."
                );

                tokio::time::sleep(delay).await;
                // Exponential backoff, max 30 seconds
                delay = std::cmp::min(delay * 2, Duration::from_secs(30));
            }
        }
    }
}

/// Run all seeders in order
/// Order: organization → users (+ default-org harnesses) → providers → models → mcp_servers
/// Organization must be seeded first (all resources have org_id FK).
/// Default-org harnesses are initialized inline by seed_anonymous_user / seed_admin_user.
/// Multi-org harness reconciliation runs post-seed via reconcile_org_harnesses.
/// Note: Agents are NOT seeded; they live as examples and are adopted on demand.
pub async fn seed_all(
    db: &StorageBackend,
    grade: DeploymentGrade,
    auth_ctx: &SeedAuthContext,
) -> anyhow::Result<SeedResult> {
    let host_composition = crate::platform::oss_host_composition_for_grade(grade);
    seed_all_with_host_composition(
        db,
        grade,
        auth_ctx,
        &host_composition,
        &crate::platform::oss_built_in_harnesses(),
    )
    .await
}

/// Run all seeders in order using an explicit platform definition.
pub async fn seed_all_with_host_composition(
    db: &StorageBackend,
    _grade: DeploymentGrade,
    auth_ctx: &SeedAuthContext,
    host_composition: &HostComposition,
    built_in_harnesses: &[crate::records::BuiltInHarnessDefinition],
) -> anyhow::Result<SeedResult> {
    let mut result = seed_auth_prerequisites(db, auth_ctx, built_in_harnesses).await?;

    // Seed providers (models depend on them)
    let provider_result = seed_providers_with_host_composition(db, host_composition).await?;
    tracing::debug!(
        created = provider_result.created,
        updated = provider_result.updated,
        unchanged = provider_result.unchanged,
        "Providers seeded"
    );
    result.merge(provider_result);

    // Seed models (depend on providers)
    let model_result = seed_models_with_host_composition(db, host_composition).await?;
    tracing::debug!(
        created = model_result.created,
        updated = model_result.updated,
        unchanged = model_result.unchanged,
        "Models seeded"
    );
    result.merge(model_result);

    // Seed MCP servers (before agents that may use them)
    let mcp_result = seed_mcp_servers(db).await?;
    tracing::debug!(
        created = mcp_result.created,
        updated = mcp_result.updated,
        unchanged = mcp_result.unchanged,
        "MCP servers seeded"
    );
    result.merge(mcp_result);

    // Default-org harnesses were initialized above by seed_anonymous_user / seed_admin_user.
    // Multi-org reconciliation (reconcile_org_harnesses) runs post-seed to cover
    // all orgs. Auth registration handlers also call initialize_org_harnesses as
    // a safety net for the race between seed task and first user registration.

    // Seed agents are available as examples (GET /v1/agent-examples) and adopted
    // on demand via POST /v1/agent-examples/{slug}/use. No automatic seeding —
    // this prevents duplicate agents when users adopt from the examples gallery.

    Ok(result)
}

#[cfg(test)]
mod model_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBackend;
    use crate::storage::models::{UpdateHarness, UpdateMcpServer, UpdateProvider};

    fn make_db() -> StorageBackend {
        StorageBackend::test_database()
    }

    fn built_in_harnesses() -> Vec<crate::records::BuiltInHarnessDefinition> {
        org_init::default_harness_definitions()
    }

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct EnvVarGuard {
        previous: Vec<(&'static str, Option<String>)>,
    }

    impl EnvVarGuard {
        fn capture(keys: &[&'static str]) -> Self {
            Self {
                previous: keys
                    .iter()
                    .map(|&key| (key, std::env::var(key).ok()))
                    .collect(),
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            for (key, value) in self.previous.drain(..) {
                match value {
                    Some(value) => unsafe { std::env::set_var(key, value) },
                    None => unsafe { std::env::remove_var(key) },
                }
            }
        }
    }

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    // --- DEFAULT_*_API_KEY materialization (single-tenant/dev) ---

    fn test_encryption() -> EncryptionService {
        EncryptionService::new("kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[]).unwrap()
    }

    fn auth_config(mode: AuthMode, disable_signup: bool) -> AuthConfig {
        AuthConfig {
            mode,
            disable_signup,
            ..AuthConfig::default()
        }
    }

    fn github_oauth_config() -> crate::auth::config::GitHubOAuthConfig {
        crate::auth::config::GitHubOAuthConfig {
            base: crate::auth::config::OAuthProviderConfig {
                client_id: "client-id".to_string(),
                client_secret: "client-secret".to_string(),
                redirect_uri: "http://localhost/callback".to_string(),
            },
        }
    }

    #[test]
    fn materialize_gate_defaults_to_dev_only() {
        // Unset: enabled in dev, disabled otherwise.
        assert!(materialize_env_provider_keys_enabled_with(
            DeploymentGrade::Dev,
            |_| None
        ));
        assert!(!materialize_env_provider_keys_enabled_with(
            DeploymentGrade::Prod,
            |_| None
        ));
        assert!(!materialize_env_provider_keys_enabled_with(
            DeploymentGrade::Preview,
            |_| None
        ));
    }

    #[test]
    fn materialize_gate_explicit_opt_in_overrides_grade() {
        // Single-tenant prod opts in explicitly.
        assert!(materialize_env_provider_keys_enabled_with(
            DeploymentGrade::Prod,
            |_| Some("true".to_string())
        ));
        assert!(materialize_env_provider_keys_enabled_with(
            DeploymentGrade::Prod,
            |_| Some("1".to_string())
        ));
        // Explicit opt-out wins even in dev.
        assert!(!materialize_env_provider_keys_enabled_with(
            DeploymentGrade::Dev,
            |_| Some("false".to_string())
        ));
    }

    #[test]
    fn materialize_guard_blocks_non_dev_builtin_self_provisioning() {
        let open_signup = auth_config(AuthMode::Full, false);
        assert!(!materialize_env_provider_keys_allowed_with(
            DeploymentGrade::Prod,
            &open_signup,
            |_| Some("true".to_string())
        ));

        let mut oauth_open = auth_config(AuthMode::Full, true);
        oauth_open.github = Some(github_oauth_config());
        assert!(!materialize_env_provider_keys_allowed_with(
            DeploymentGrade::Prod,
            &oauth_open,
            |_| Some("true".to_string())
        ));
    }

    #[test]
    fn materialize_guard_allows_non_dev_when_builtin_self_provisioning_closed() {
        let closed_full = auth_config(AuthMode::Full, true);
        assert!(materialize_env_provider_keys_allowed_with(
            DeploymentGrade::Prod,
            &closed_full,
            |_| Some("true".to_string())
        ));

        let admin_only = auth_config(AuthMode::Admin, false);
        assert!(materialize_env_provider_keys_allowed_with(
            DeploymentGrade::Prod,
            &admin_only,
            |_| Some("true".to_string())
        ));
    }

    #[test]
    fn materialize_guard_preserves_dev_convenience_default() {
        let open_signup = auth_config(AuthMode::Full, false);
        assert!(materialize_env_provider_keys_allowed_with(
            DeploymentGrade::Dev,
            &open_signup,
            |_| None
        ));
    }

    /// Materialization fills an empty default-org provider with the env key,
    /// and the key is resolvable through the normal fail-closed DB path.
    #[tokio::test]
    async fn materialize_fills_empty_provider_and_resolves() {
        let db = make_db();
        let encryption = test_encryption();
        let platform = crate::platform::oss_host_composition_for_grade(DeploymentGrade::Dev);

        // Seed the bare provider rows (no keys).
        seed_providers_with_host_composition(&db, &platform)
            .await
            .unwrap();

        let result = seed_default_provider_keys_with_lookup(&db, &encryption, &platform, |ty| {
            (ty == "openai").then(|| "sk-from-env".to_string())
        })
        .await
        .unwrap();
        assert_eq!(result.updated, 1, "exactly the openai provider is filled");

        // The key round-trips through the same decrypt path the resolver uses.
        let provider = db
            .get_provider(DEFAULT_ORG_ID, seed_ids::OPENAI_PROVIDER)
            .await
            .unwrap()
            .expect("openai provider seeded");
        assert!(provider.api_key_set);
        let resolved = crate::services::provider_resolver::resolve_provider_api_key(
            &db,
            Some(&encryption),
            &provider,
        )
        .unwrap();
        assert_eq!(resolved, Some("sk-from-env".to_string()));
    }

    /// Materialization never overwrites an explicitly-configured key.
    #[tokio::test]
    async fn materialize_does_not_clobber_existing_key() {
        let db = make_db();
        let encryption = test_encryption();
        let platform = crate::platform::oss_host_composition_for_grade(DeploymentGrade::Dev);
        seed_providers_with_host_composition(&db, &platform)
            .await
            .unwrap();

        // User configures their own key.
        let user_key = encryption.encrypt_string("sk-user-set").unwrap();
        db.update_provider(
            DEFAULT_ORG_ID,
            seed_ids::OPENAI_PROVIDER,
            UpdateProvider {
                name: None,
                provider_type: None,
                base_url: None,
                api_key_encrypted: Some(user_key),
                status: None,
                settings: None,
            },
        )
        .await
        .unwrap();

        let result = seed_default_provider_keys_with_lookup(&db, &encryption, &platform, |ty| {
            (ty == "openai").then(|| "sk-from-env".to_string())
        })
        .await
        .unwrap();
        assert_eq!(result.updated, 0);
        assert_eq!(result.unchanged, 1);

        let provider = db
            .get_provider(DEFAULT_ORG_ID, seed_ids::OPENAI_PROVIDER)
            .await
            .unwrap()
            .unwrap();
        let resolved = crate::services::provider_resolver::resolve_provider_api_key(
            &db,
            Some(&encryption),
            &provider,
        )
        .unwrap();
        assert_eq!(
            resolved,
            Some("sk-user-set".to_string()),
            "user key must be preserved"
        );
    }

    /// With no env key set, nothing is materialized (multitenant-safe default
    /// even if the gate were somehow on).
    #[tokio::test]
    async fn materialize_no_env_key_is_noop() {
        let db = make_db();
        let encryption = test_encryption();
        let platform = crate::platform::oss_host_composition_for_grade(DeploymentGrade::Dev);
        seed_providers_with_host_composition(&db, &platform)
            .await
            .unwrap();

        let result = seed_default_provider_keys_with_lookup(&db, &encryption, &platform, |_| None)
            .await
            .unwrap();
        assert_eq!(result.updated, 0);

        let provider = db
            .get_provider(DEFAULT_ORG_ID, seed_ids::OPENAI_PROVIDER)
            .await
            .unwrap()
            .unwrap();
        assert!(!provider.api_key_set);
    }

    // --- Harness seed data ---

    #[test]
    fn test_seed_harness_names_are_unique() {
        let built_in_harnesses = built_in_harnesses();
        let names: Vec<&str> = built_in_harnesses.iter().map(|h| h.name.as_str()).collect();
        let mut unique_names = names.clone();
        unique_names.sort();
        unique_names.dedup();
        assert_eq!(
            names.len(),
            unique_names.len(),
            "Seed harness names must be unique"
        );
    }

    #[test]
    fn test_base_harness_has_no_capabilities() {
        let built_in_harnesses = built_in_harnesses();
        let base = built_in_harnesses
            .iter()
            .find(|h| h.name == "base")
            .expect("Base harness should exist");
        assert!(
            base.capabilities.is_empty(),
            "Base harness must have no capabilities"
        );
        assert!(base.tags.iter().any(|tag| tag == "base"));
    }

    #[test]
    fn test_generic_harness_has_expected_capabilities() {
        let built_in_harnesses = built_in_harnesses();
        let generic = built_in_harnesses
            .iter()
            .find(|h| h.name == "generic")
            .expect("Generic harness should exist");

        let cap_ids: Vec<&str> = generic
            .capabilities
            .iter()
            .map(|c| c.capability_id())
            .collect();
        assert_eq!(cap_ids.len(), 25);
        assert!(cap_ids.contains(&"human_intent"));
        assert!(cap_ids.contains(&"session_file_system"));
        assert!(cap_ids.contains(&"bashkit_shell"));
        assert!(cap_ids.contains(&"web_fetch"));
        assert!(cap_ids.contains(&"session_storage"));
        assert!(cap_ids.contains(&"session"));
        assert!(cap_ids.contains(&"session_schedule"));
        assert!(cap_ids.contains(&"btw"));
        assert!(cap_ids.contains(&"agent_instructions"));
        assert!(cap_ids.contains(&"skills"));
        assert!(cap_ids.contains(&"infinity_context"));
        assert!(cap_ids.contains(&"auto_tool_search"));
        assert!(cap_ids.contains(&"budgeting"));
        assert!(cap_ids.contains(&"self_budget"));
        assert!(cap_ids.contains(&"loop_detection"));
        assert!(cap_ids.contains(&"error_disclosure"));
        assert!(cap_ids.contains(&"message_metadata"));
        assert!(cap_ids.contains(&"compaction"));
        assert!(cap_ids.contains(&"tool_output_persistence"));
        assert!(cap_ids.contains(&"tool_output_distillation"));
        assert!(cap_ids.contains(&"parallel_tool_calls"));
        assert!(cap_ids.contains(&"citation_retrieval"));
        assert!(cap_ids.contains(&"citation_verification"));
        assert!(cap_ids.contains(&"soft_approval"));
        // Verify compaction default config
        let compaction_cap = generic
            .capabilities
            .iter()
            .find(|c| c.capability_id() == "compaction")
            .expect("compaction capability should exist");
        assert_eq!(compaction_cap.config_value()["strategy"], "auto");
        assert_eq!(compaction_cap.config_value()["proactive"], true);
        assert_eq!(compaction_cap.config_value()["budget_percent"], 0.85);
        // The trusted default harness opts into detailed error disclosure.
        let error_disclosure_cap = generic
            .capabilities
            .iter()
            .find(|c| c.capability_id() == "error_disclosure")
            .expect("error_disclosure capability should exist");
        assert_eq!(error_disclosure_cap.config_value()["mode"], "detailed");
        assert!(generic.tags.iter().any(|tag| tag == "generic"));
        assert!(generic.tags.iter().any(|tag| tag == "deprecated"));
    }

    #[test]
    fn test_generic_harness_capabilities_are_registered() {
        // Verify all capability IDs referenced by Generic harness exist in the registry
        let registry =
            crate::platform::oss_capability_registry_for_grade(everruns_core::DeploymentGrade::Dev);

        let built_in_harnesses = built_in_harnesses();
        let generic = built_in_harnesses
            .iter()
            .find(|h| h.name == "generic")
            .expect("Generic harness should exist");

        for cap in &generic.capabilities {
            assert!(
                registry.has(cap.capability_id()),
                "Capability '{}' referenced by Generic harness must be registered",
                cap.capability_id()
            );
        }
    }

    #[test]
    fn test_provider_neutral_coding_example_capabilities_are_registered() {
        let registry =
            crate::platform::oss_capability_registry_for_grade(everruns_core::DeploymentGrade::Dev);

        let example = crate::harnesses::find_harness_example("coding")
            .expect("coding should exist in the example catalogue");

        for cap in &example.definition.capabilities {
            assert!(
                registry.has(cap.capability_id()),
                "Capability '{}' referenced by Coding example must be registered",
                cap.capability_id()
            );
        }
    }

    #[test]
    fn test_coding_container_capability_unregistered_when_flag_disabled() {
        let _lock = lock_env();
        let _env_guard =
            EnvVarGuard::capture(&["FEATURE_CONTAINER_SANDBOX", "FEATURE_DOCKER_CAPABILITY"]);
        unsafe { std::env::remove_var("FEATURE_CONTAINER_SANDBOX") };
        unsafe { std::env::remove_var("FEATURE_DOCKER_CAPABILITY") };

        let registry =
            crate::platform::oss_capability_registry_for_grade(everruns_core::DeploymentGrade::Dev);

        // Container execution remains feature-gated even though provider
        // selection no longer creates a provider-specific coding harness.
        assert!(
            !registry.has("container_sandbox"),
            "container_sandbox capability should not be registered when feature flag is off"
        );

        // Provider-specific coding harnesses remain released legacy rows only.
        let built_in_harnesses = built_in_harnesses();
        assert!(
            built_in_harnesses
                .iter()
                .all(|h| !h.name.starts_with("coding-")),
            "provider-specific coding harnesses are no longer default built-ins"
        );
    }

    /// Regression test for "Tool not found: bash" bug.
    ///
    /// When a session uses the Generic Harness without an agent, the worker must
    /// build a ToolRegistry from harness capabilities. This test verifies that
    /// collect_capabilities on the Generic Harness cap IDs actually produces a
    /// registry containing the 'bash' tool — the exact path that was broken.
    #[tokio::test]
    async fn test_generic_harness_capabilities_produce_bash_tool() {
        use everruns_core::capabilities::{SystemPromptContext, collect_capabilities};

        let registry =
            crate::platform::oss_capability_registry_for_grade(everruns_core::DeploymentGrade::Dev);

        let built_in_harnesses = built_in_harnesses();
        let generic = built_in_harnesses
            .iter()
            .find(|h| h.name == "generic")
            .expect("Generic harness should exist");

        let cap_ids: Vec<String> = generic
            .capabilities
            .iter()
            .map(|s| s.capability_id().to_string())
            .collect();
        let ctx =
            SystemPromptContext::without_file_store(everruns_contracts::typed_id::SessionId::new());
        let collected = collect_capabilities(&cap_ids, &registry, &ctx).await;

        // Build a ToolRegistry exactly as the worker does
        let mut tool_registry = everruns_core::ToolRegistry::with_defaults();
        for tool in collected.tools {
            tool_registry.register_boxed(tool);
        }

        assert!(
            tool_registry.has("bash"),
            "ToolRegistry built from Generic Harness capabilities must include 'bash' tool. \
             This was the root cause of the 'Tool not found: bash' bug."
        );
    }

    /// Verify that ToolRegistry::with_defaults() alone does NOT include 'bash'.
    /// This documents why harness capability registration is necessary.
    #[test]
    fn test_defaults_alone_miss_bash_tool() {
        let registry = everruns_core::ToolRegistry::with_defaults();
        assert!(
            !registry.has("bash"),
            "with_defaults() must NOT include 'bash' — it comes from bashkit_shell capability. \
             If this fails, the tool was added to defaults and the harness fallback is moot."
        );
    }

    /// Verify server-executed Generic tools have implementations.
    #[tokio::test]
    async fn test_generic_harness_collected_tools_have_implementations() {
        use everruns_contracts::tool_types::ToolPolicy;
        use everruns_core::capabilities::{SystemPromptContext, collect_capabilities};

        let registry =
            crate::platform::oss_capability_registry_for_grade(everruns_core::DeploymentGrade::Dev);

        let built_in_harnesses = built_in_harnesses();
        let generic = built_in_harnesses
            .iter()
            .find(|h| h.name == "generic")
            .expect("Generic harness should exist");

        let cap_ids: Vec<String> = generic
            .capabilities
            .iter()
            .map(|s| s.capability_id().to_string())
            .collect();
        let ctx =
            SystemPromptContext::without_file_store(everruns_contracts::typed_id::SessionId::new());
        let collected = collect_capabilities(&cap_ids, &registry, &ctx).await;

        // Client-side definitions deliberately park for an external result.
        let builtin_definition_count = collected
            .tool_definitions
            .iter()
            .filter(|definition| definition.policy() != &ToolPolicy::ClientSide)
            .count();
        assert_eq!(
            collected.tools.len(),
            builtin_definition_count,
            "tool implementations ({}) must match built-in tool definitions ({}) — \
             mismatches cause 'Tool not found' at runtime",
            collected.tools.len(),
            builtin_definition_count,
        );

        let tool_names: Vec<&str> = collected
            .tool_definitions
            .iter()
            .map(|t| t.name())
            .collect();
        assert!(
            ["bash", "list_skills", "activate_skill", "ask_user"]
                .iter()
                .all(|expected| tool_names.contains(expected))
        );
    }

    /// Verify Generic Harness includes skills discovery tools.
    #[tokio::test]
    async fn test_generic_harness_capabilities_produce_skills_tools() {
        use everruns_core::capabilities::{SystemPromptContext, collect_capabilities};

        let registry =
            crate::platform::oss_capability_registry_for_grade(everruns_core::DeploymentGrade::Dev);

        let built_in_harnesses = built_in_harnesses();
        let generic = built_in_harnesses
            .iter()
            .find(|h| h.name == "generic")
            .expect("Generic harness should exist");

        let cap_ids: Vec<String> = generic
            .capabilities
            .iter()
            .map(|s| s.capability_id().to_string())
            .collect();
        let ctx =
            SystemPromptContext::without_file_store(everruns_contracts::typed_id::SessionId::new());
        let collected = collect_capabilities(&cap_ids, &registry, &ctx).await;

        let tool_names: Vec<&str> = collected
            .tool_definitions
            .iter()
            .map(|t| t.name())
            .collect();

        assert!(
            tool_names.contains(&"list_skills"),
            "Generic Harness must include 'list_skills' tool from skills capability, got: {:?}",
            tool_names
        );
        assert!(
            tool_names.contains(&"activate_skill"),
            "Generic Harness must include 'activate_skill' tool from skills capability, got: {:?}",
            tool_names
        );
    }

    // --- seed_all ---

    #[tokio::test]
    async fn test_seed_all_seeds_default_marketplace() {
        use everruns_core::DEFAULT_ORG_ID;
        let db = make_db();
        seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();

        let marketplaces = db
            .list_plugin_marketplaces(DEFAULT_ORG_ID, None)
            .await
            .unwrap();
        assert!(
            marketplaces.iter().any(|m| m.name == "everruns"),
            "seed_all must create the default 'everruns' marketplace; got: {:?}",
            marketplaces.iter().map(|m| &m.name).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn test_seed_all_first_run_creates_everything() {
        let db = make_db();
        let result = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();

        assert!(result.created > 0, "first run should create items");
        assert_eq!(result.updated, 0, "first run should not update anything");

        // seed_all creates default-org harnesses via seed_anonymous_user.
        // Run reconciliation too, matching the full startup flow.
        reconcile_org_harnesses(&db, &crate::platform::oss_built_in_harnesses()).await;

        let settings = db
            .get_organization_settings(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .unwrap();
        let harnesses = db
            .list_harnesses(DEFAULT_ORG_ID, None, false)
            .await
            .unwrap();
        let conversation_id = harnesses
            .iter()
            .find(|h| h.name == "conversation")
            .expect("conversation harness")
            .id;
        let base_id = harnesses
            .iter()
            .find(|h| h.name == "base")
            .expect("base harness")
            .id;
        assert_eq!(settings.default_harness_id, Some(conversation_id));
        assert_eq!(settings.base_harness_id, Some(base_id));
        assert_eq!(settings.default_model_id, None);
    }

    #[tokio::test]
    async fn test_seed_all_second_run_all_unchanged() {
        let db = make_db();
        let _ = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();

        let result = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();
        assert_eq!(result.created, 0, "second run should create nothing");
        assert_eq!(result.updated, 0, "second run should update nothing");
        assert!(result.unchanged > 0, "everything should be unchanged");
    }

    #[tokio::test]
    async fn test_seed_all_has_changes() {
        let db = make_db();
        let first = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();
        assert!(first.has_changes());

        let second = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();
        assert!(!second.has_changes());
    }

    #[tokio::test]
    async fn test_seed_all_admin_mode_existing_email_is_idempotent() {
        let db = make_db();

        let existing_admin = db
            .create_user(CreateUserRow {
                email: "admin@example.com".to_string(),
                name: "Existing Admin".to_string(),
                avatar_url: None,
                roles: vec!["admin".to_string()],
                password_hash: None,
                email_verified: true,
                auth_provider: Some("local".to_string()),
                auth_provider_id: None,
                external_id: None,
            })
            .await
            .unwrap();

        let result = seed_all(
            &db,
            DeploymentGrade::Dev,
            &SeedAuthContext {
                mode: AuthMode::Admin,
                admin: Some(AdminConfig {
                    email: "admin@example.com".to_string(),
                    password: "password123".to_string(),
                }),
            },
        )
        .await
        .unwrap();

        // seed_all creates many other entities (harnesses, agents, providers) on
        // first run, so we don't assert `result.created == 0` here — the test
        // just guarantees the admin user isn't recreated and its identity is
        // preserved.
        let _ = result;

        let admin_by_email = db
            .get_user_by_email("admin@example.com")
            .await
            .unwrap()
            .expect("admin user should exist");
        assert_eq!(
            admin_by_email.id, existing_admin.id,
            "seeding should preserve existing admin identity"
        );

        let membership = db
            .get_organization_member(DEFAULT_ORG_ID, existing_admin.id)
            .await
            .unwrap()
            .expect("admin should be added to default org");
        assert_eq!(membership.role, "owner");
    }

    #[tokio::test]
    async fn test_seed_all_admin_mode_existing_non_admin_email_is_rejected() {
        let db = make_db();

        let existing_user = db
            .create_user(CreateUserRow {
                email: "admin@example.com".to_string(),
                name: "Regular User".to_string(),
                avatar_url: None,
                roles: vec!["user".to_string()],
                password_hash: None,
                email_verified: false,
                auth_provider: Some("local".to_string()),
                auth_provider_id: None,
                external_id: None,
            })
            .await
            .unwrap();

        let result = seed_all(
            &db,
            DeploymentGrade::Dev,
            &SeedAuthContext {
                mode: AuthMode::Admin,
                admin: Some(AdminConfig {
                    email: "admin@example.com".to_string(),
                    password: "password123".to_string(),
                }),
            },
        )
        .await;

        assert!(
            result.is_err(),
            "existing untrusted account must not be promoted to owner"
        );

        let membership = db
            .get_organization_member(DEFAULT_ORG_ID, existing_user.id)
            .await
            .unwrap();
        assert!(
            membership.is_none(),
            "non-admin user should not be added to org"
        );
    }

    // --- Agent seeding regression ---

    #[tokio::test]
    async fn test_seed_all_creates_only_the_managed_platform_agent() {
        // Agents should NOT be auto-seeded; they live as examples and are adopted on demand.
        let db = make_db();
        seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();

        let (agents, _) = db
            .list_agents(
                DEFAULT_ORG_ID,
                None,
                false,
                crate::api::common::Pagination::new(0, 100),
            )
            .await
            .unwrap();
        assert_eq!(agents.len(), 1);
        assert!(agents[0].is_built_in);
        assert_eq!(agents[0].name, "platform-chat");
    }

    // --- Provider upsert ---

    #[tokio::test]
    async fn test_provider_seed_detects_name_change() {
        let db = make_db();
        let _ = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();

        // Mutate provider name via public API
        db.update_provider(
            DEFAULT_ORG_ID,
            seed_ids::OPENAI_PROVIDER,
            UpdateProvider {
                name: Some("STALE".to_string()),
                provider_type: None,
                base_url: None,
                api_key_encrypted: None,
                status: None,
                settings: None,
            },
        )
        .await
        .unwrap();

        let result = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();
        assert!(result.updated >= 1, "should detect provider name change");
    }

    // --- MCP Server upsert ---

    #[tokio::test]
    async fn linear_mcp_seed_uses_application_actor_oauth() {
        let db = make_db();
        seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();

        let row = db
            .get_mcp_server(DEFAULT_ORG_ID, seed_ids::LINEAR_MCP)
            .await
            .unwrap()
            .expect("Linear MCP preset should be seeded");
        let settings = crate::domains::mcp_servers::McpServerService::settings_from_row(&row);
        let oauth = settings
            .oauth
            .as_ref()
            .expect("Linear MCP preset should use OAuth");

        assert_eq!(row.name, "linear");
        assert_eq!(row.url, "https://mcp.linear.app/mcp");
        assert_eq!(settings.auth_mode, everruns_core::McpServerAuthMode::OAuth);
        assert_eq!(settings.protocol_mode, everruns_core::McpProtocolMode::Auto);
        assert_eq!(oauth.scope.as_deref(), Some("read,write"));
        assert_eq!(
            oauth
                .service_authorization_params
                .get("actor")
                .map(String::as_str),
            Some("app")
        );
    }

    #[tokio::test]
    async fn test_mcp_server_seed_detects_url_change() {
        let db = make_db();
        let _ = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();

        // Mutate MCP server URL via public API
        db.update_mcp_server(
            DEFAULT_ORG_ID,
            seed_ids::MS_LEARN_MCP,
            UpdateMcpServer {
                url: Some("https://old.example.com".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let result = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();
        assert!(result.updated >= 1, "should detect MCP server URL change");
    }

    // --- Harness upsert ---

    #[tokio::test]
    async fn test_harness_reconcile_detects_description_change() {
        let db = make_db();
        let _ = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();
        reconcile_org_harnesses(&db, &crate::platform::oss_built_in_harnesses()).await;

        // Mutate harness description via public API. Resolve the base
        // harness id by name from the DB — built-in harnesses no longer
        // carry compile-time UUIDs.
        let harness_id = db
            .get_harness_by_name(DEFAULT_ORG_ID, "base")
            .await
            .unwrap()
            .expect("base harness should be provisioned")
            .id;
        db.update_harness(
            DEFAULT_ORG_ID,
            harness_id,
            UpdateHarness {
                description: Some("STALE".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        // Reconciliation should detect the stale description and update it
        let result = org_init::initialize_org_harnesses(&db, DEFAULT_ORG_ID)
            .await
            .unwrap();
        assert!(
            result.updated >= 1,
            "should detect harness description change"
        );
    }

    // --- SeedResult ---

    #[test]
    fn test_seed_result_merge() {
        let mut a = SeedResult {
            created: 1,
            updated: 2,
            unchanged: 3,
        };
        let b = SeedResult {
            created: 10,
            updated: 20,
            unchanged: 30,
        };
        a.merge(b);
        assert_eq!(a.created, 11);
        assert_eq!(a.updated, 22);
        assert_eq!(a.unchanged, 33);
    }

    #[test]
    fn test_seed_result_has_changes() {
        assert!(!SeedResult::default().has_changes());
        assert!(
            SeedResult {
                created: 1,
                updated: 0,
                unchanged: 0
            }
            .has_changes()
        );
        assert!(
            SeedResult {
                created: 0,
                updated: 1,
                unchanged: 0
            }
            .has_changes()
        );
        assert!(
            !SeedResult {
                created: 0,
                updated: 0,
                unchanged: 5
            }
            .has_changes()
        );
    }

    // --- Multi-org harness reconciliation ---

    #[tokio::test]
    async fn test_reconcile_org_harnesses() {
        use crate::storage::models::CreateOrganizationRow;

        let db = make_db();
        // Seed creates the org but no longer initialises harnesses inline.
        seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();

        // Create a second org.
        let second_org = db
            .create_organization(CreateOrganizationRow {
                public_id: format!("org_{}", uuid::Uuid::now_v7().simple()),
                name: "Second Org".into(),
                created_by: None,
            })
            .await
            .unwrap();

        // Before reconciliation, second org should have no harnesses.
        let before = db
            .list_harnesses(second_org.org_id, None, false)
            .await
            .unwrap();
        assert!(
            before.is_empty(),
            "second org should start with no harnesses"
        );

        // Run reconciliation (covers ALL orgs, including default).
        reconcile_org_harnesses(&db, &crate::platform::oss_built_in_harnesses()).await;

        // After reconciliation, both orgs should have the built-in harnesses.
        let default_harnesses = db
            .list_harnesses(DEFAULT_ORG_ID, None, false)
            .await
            .unwrap();
        assert!(
            !default_harnesses.is_empty(),
            "default org should have harnesses after reconciliation"
        );

        let second_harnesses = db
            .list_harnesses(second_org.org_id, None, false)
            .await
            .unwrap();
        assert_eq!(second_harnesses.len(), default_harnesses.len());
    }
}
