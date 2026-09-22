//! Built-in LLM model catalogue seeded into the default org's providers.
//!
//! Split out of `seed.rs` to keep that file under its size ratchet; the
//! seeding logic that consumes `SEED_MODELS` stays there.

use super::seed_ids;
use uuid::Uuid;

/// Seed LLM model definition
pub(super) struct SeedModel {
    pub(super) id: Uuid,
    pub(super) provider_id: Uuid,
    pub(super) model_id: &'static str,
    pub(super) display_name: &'static str,
    pub(super) enabled: bool,
    pub(super) is_favorite: bool,
}

/// Built-in seed models
pub(super) const SEED_MODELS: &[SeedModel] = &[
    // OpenAI embeddings. Knowledge indexes require a concrete embedding model,
    // so the default catalog must contain one even before provider discovery.
    SeedModel {
        id: seed_ids::TEXT_EMBEDDING_3_SMALL,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "text-embedding-3-small",
        display_name: "Text Embedding 3 Small",
        enabled: true,
        is_favorite: true,
    },
    // OpenAI Realtime series
    SeedModel {
        id: seed_ids::GPT_REALTIME_2,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-realtime-2",
        display_name: "GPT Realtime 2",
        enabled: false,
        is_favorite: false,
    },
    // OpenAI GPT-5.6 series (Sol / Terra / Luna)
    SeedModel {
        id: seed_ids::GPT_5_6_SOL,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.6-sol",
        display_name: "GPT-5.6 Sol",
        enabled: true,     // Enabled by default
        is_favorite: true, // Favorite model
    },
    SeedModel {
        // Platform default model (see `platform::PLATFORM_DEFAULT_MODEL_ID`).
        id: seed_ids::GPT_5_6_TERRA,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.6-terra",
        display_name: "GPT-5.6 Terra",
        enabled: true,     // Enabled by default
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GPT_5_6_LUNA,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.6-luna",
        display_name: "GPT-5.6 Luna",
        enabled: true, // Enabled by default
        is_favorite: false,
    },
    SeedModel {
        // GPT-6 Astra is OpenAI's current flagship (above the GPT-5.6 series).
        // GPT-5.6 Terra stays the platform default for now; Astra is available
        // as an enabled favorite for the hardest reasoning/agentic work.
        id: seed_ids::GPT_6_ASTRA,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-6-astra",
        display_name: "GPT-6 Astra",
        enabled: true,     // Enabled by default
        is_favorite: true, // Favorite model
    },
    // OpenAI GPT-5.5 series
    SeedModel {
        id: seed_ids::GPT_5_5,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.5",
        display_name: "GPT-5.5",
        enabled: true,     // Enabled by default
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GPT_5_5_PRO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.5-pro",
        display_name: "GPT-5.5 Pro",
        enabled: false,
        is_favorite: true, // Favorite model
    },
    // OpenAI GPT-5.4 series
    SeedModel {
        id: seed_ids::GPT_5_4,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.4",
        display_name: "GPT-5.4",
        enabled: true,     // Enabled by default
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GPT_5_4_PRO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.4-pro",
        display_name: "GPT-5.4 Pro",
        enabled: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GPT_5_4_MINI,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.4-mini",
        display_name: "GPT-5.4 mini",
        enabled: true,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GPT_5_4_NANO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.4-nano",
        display_name: "GPT-5.4 nano",
        enabled: false,
        is_favorite: false,
    },
    // OpenAI GPT-5.2 series
    SeedModel {
        id: seed_ids::GPT_5_2,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.2",
        display_name: "GPT-5.2",
        enabled: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GPT_5_2_PRO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.2-pro",
        display_name: "GPT-5.2 Pro",
        enabled: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GPT_5_2_CODEX,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.2-codex",
        display_name: "GPT-5.2 Codex",
        enabled: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GPT_5_2_CHAT,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.2-chat-latest",
        display_name: "GPT-5.2 Chat",
        enabled: false,
        is_favorite: false,
    },
    // OpenAI GPT-5.1 series
    SeedModel {
        id: seed_ids::GPT_5_1,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.1",
        display_name: "GPT-5.1",
        enabled: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_1_CODEX,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.1-codex",
        display_name: "GPT-5.1 Codex",
        enabled: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_1_CODEX_MINI,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.1-codex-mini",
        display_name: "GPT-5.1 Codex mini",
        enabled: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_1_CODEX_MAX,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.1-codex-max",
        display_name: "GPT-5.1 Codex max",
        enabled: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_1_CHAT,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.1-chat-latest",
        display_name: "GPT-5.1 Chat",
        enabled: false,
        is_favorite: false,
    },
    // OpenAI GPT-5 series
    SeedModel {
        id: seed_ids::GPT_5_MINI,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5-mini",
        display_name: "GPT-5 mini",
        enabled: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5",
        display_name: "GPT-5",
        enabled: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_NANO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5-nano",
        display_name: "GPT-5 nano",
        enabled: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_PRO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5-pro",
        display_name: "GPT-5 Pro",
        enabled: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_CODEX,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5-codex",
        display_name: "GPT-5 Codex",
        enabled: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_CHAT,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5-chat-latest",
        display_name: "GPT-5 Chat",
        enabled: false,
        is_favorite: false,
    },
    // OpenAI GPT-4.1 series
    SeedModel {
        id: seed_ids::GPT_4_1,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-4.1",
        display_name: "GPT-4.1",
        enabled: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_4_1_MINI,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-4.1-mini",
        display_name: "GPT-4.1 mini",
        enabled: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_4_1_NANO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-4.1-nano",
        display_name: "GPT-4.1 nano",
        enabled: false,
        is_favorite: false,
    },
    // OpenAI Reasoning models (o-series)
    SeedModel {
        id: seed_ids::O4_MINI,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o4-mini",
        display_name: "o4 mini",
        enabled: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O4_MINI_DEEP_RESEARCH,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o4-mini-deep-research",
        display_name: "o4 mini Deep Research",
        enabled: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O3,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o3",
        display_name: "o3",
        enabled: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O3_PRO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o3-pro",
        display_name: "o3 Pro",
        enabled: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O3_DEEP_RESEARCH,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o3-deep-research",
        display_name: "o3 Deep Research",
        enabled: false,
        is_favorite: false,
    },
    // Anthropic current-gen (Fable 5.1, Opus 5.5, Opus 5, Sonnet 5, Opus 4.8)
    SeedModel {
        // Fable 5.1 is Anthropic's top tier above Opus (successor to Fable 5,
        // which is intentionally not seeded). Priced well above Opus 5.5, so Opus 5.5
        // stays the recommended default while Fable 5.1 is available for the
        // hardest reasoning and long-horizon agentic work.
        id: seed_ids::CLAUDE_FABLE_5_1,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-fable-5-1",
        display_name: "Claude Fable 5.1",
        enabled: true,     // Enabled by default
        is_favorite: true, // Favorite model
    },
    SeedModel {
        // 1M-context twin of the 200K base above (driver sends the `context-1m`
        // beta header for `[1m]` ids).
        id: seed_ids::CLAUDE_FABLE_5_1_1M,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-fable-5-1[1m]",
        display_name: "Claude Fable 5.1 (1M)",
        enabled: true,     // Enabled by default
        is_favorite: true, // Favorite model
    },
    SeedModel {
        // Opus 5.5 is the current Opus flagship — the recommended Anthropic model.
        id: seed_ids::CLAUDE_OPUS_5_5,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-opus-5-5",
        display_name: "Claude Opus 5.5",
        enabled: true,     // Enabled by default
        is_favorite: true, // Favorite model
    },
    SeedModel {
        // 1M-context twin of the 200K base above (driver sends the `context-1m`
        // beta header for `[1m]` ids).
        id: seed_ids::CLAUDE_OPUS_5_5_1M,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-opus-5-5[1m]",
        display_name: "Claude Opus 5.5 (1M)",
        enabled: true,     // Enabled by default
        is_favorite: true, // Favorite model
    },
    SeedModel {
        // Opus 5 is the previous Opus flagship, kept enabled for existing agents.
        id: seed_ids::CLAUDE_OPUS_5,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-opus-5",
        display_name: "Claude Opus 5",
        enabled: true,     // Enabled by default
        is_favorite: true, // Favorite model
    },
    SeedModel {
        // 1M-context twin of the 200K base above (driver sends the `context-1m`
        // beta header for `[1m]` ids). Keeps a 1M Opus in the default picker
        // alongside the bare `claude-opus-5` 200K variant.
        id: seed_ids::CLAUDE_OPUS_5_1M,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-opus-5[1m]",
        display_name: "Claude Opus 5 (1M)",
        enabled: true,     // Enabled by default
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::CLAUDE_OPUS_4_8,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-opus-4-8",
        display_name: "Claude Opus 4.8",
        enabled: true,     // Enabled by default
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::CLAUDE_SONNET_5,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-sonnet-5",
        display_name: "Claude Sonnet 5",
        enabled: true,     // Enabled by default
        is_favorite: true, // Favorite model
    },
    SeedModel {
        // 1M-context twin of the 200K base above (driver sends the `context-1m`
        // beta header for `[1m]` ids). Keeps a 1M Sonnet in the default picker
        // now that the bare `claude-sonnet-5` profile is the 200K variant.
        id: seed_ids::CLAUDE_SONNET_5_1M,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-sonnet-5[1m]",
        display_name: "Claude Sonnet 5 (1M)",
        enabled: true,     // Enabled by default
        is_favorite: true, // Favorite model
    },
    // Anthropic Claude 4.7 / 4.6 series
    SeedModel {
        id: seed_ids::CLAUDE_OPUS_4_7,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-opus-4-7",
        display_name: "Claude Opus 4.7",
        enabled: true,     // Enabled by default
        is_favorite: true, // Favorite model
    },
    SeedModel {
        // 1M-context twin of the 200K base above (driver sends the `context-1m`
        // beta header for `[1m]` ids). Keeps a 1M Opus in the default picker
        // now that the bare `claude-opus-4-7` profile is the 200K variant.
        id: seed_ids::CLAUDE_OPUS_4_7_1M,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-opus-4-7[1m]",
        display_name: "Claude Opus 4.7 (1M)",
        enabled: true,     // Enabled by default
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::CLAUDE_SONNET_4_6,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-sonnet-4-6",
        display_name: "Claude Sonnet 4.6",
        enabled: false,    // Superseded by Sonnet 5
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::CLAUDE_HAIKU_4_6,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-haiku-4-6-20260301",
        display_name: "Claude Haiku 4.6",
        enabled: true,     // Enabled by default
        is_favorite: true, // Favorite model
    },
    // Anthropic Claude 4.5 series
    SeedModel {
        id: seed_ids::CLAUDE_OPUS_4_5,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-opus-4-5",
        display_name: "Claude Opus 4.5",
        enabled: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::CLAUDE_SONNET_4_5,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-sonnet-4-5",
        display_name: "Claude Sonnet 4.5",
        enabled: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::CLAUDE_HAIKU_4_5,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-haiku-4-5",
        display_name: "Claude Haiku 4.5",
        enabled: false,
        is_favorite: true, // Favorite model
    },
    // Anthropic Claude 4 series
    SeedModel {
        id: seed_ids::CLAUDE_OPUS_4,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-opus-4",
        display_name: "Claude Opus 4",
        enabled: false,
        is_favorite: false,
    },
    // Google Gemini 3.x series (current gen)
    SeedModel {
        id: seed_ids::GEMINI_31_PRO_PREVIEW,
        provider_id: seed_ids::GEMINI_PROVIDER,
        model_id: "gemini-3.1-pro-preview",
        display_name: "Gemini 3.1 Pro Preview",
        enabled: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GEMINI_35_FLASH,
        provider_id: seed_ids::GEMINI_PROVIDER,
        model_id: "gemini-3.5-flash",
        display_name: "Gemini 3.5 Flash",
        enabled: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GEMINI_31_FLASH_LITE,
        provider_id: seed_ids::GEMINI_PROVIDER,
        model_id: "gemini-3.1-flash-lite",
        display_name: "Gemini 3.1 Flash Lite",
        enabled: false,
        is_favorite: false,
    },
    // Google Gemini 2.x series (superseded)
    SeedModel {
        id: seed_ids::GEMINI_25_PRO,
        provider_id: seed_ids::GEMINI_PROVIDER,
        model_id: "gemini-2.5-pro",
        display_name: "Gemini 2.5 Pro",
        enabled: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GEMINI_25_FLASH,
        provider_id: seed_ids::GEMINI_PROVIDER,
        model_id: "gemini-2.5-flash",
        display_name: "Gemini 2.5 Flash",
        enabled: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GEMINI_20_FLASH,
        provider_id: seed_ids::GEMINI_PROVIDER,
        model_id: "gemini-2.0-flash",
        display_name: "Gemini 2.0 Flash",
        enabled: false,
        is_favorite: false,
    },
    // AWS Bedrock (ConverseStream API).
    //
    // Current Claude models have no in-region Bedrock availability at all — only
    // the geo (`us.`/`eu.`/`au.`) and global cross-region inference profiles — so
    // the bare `anthropic.claude-*` ids would fail everywhere. `global.` is the
    // one prefix offered in every commercial region, which makes it the only
    // sound default for a seed. Accounts with data-residency requirements (or on
    // GovCloud, where global is unavailable) should add the matching geo id
    // instead.
    SeedModel {
        id: seed_ids::BEDROCK_CLAUDE_OPUS_5_5,
        provider_id: seed_ids::BEDROCK_PROVIDER,
        model_id: "global.anthropic.claude-opus-5-5",
        display_name: "Claude Opus 5.5 (Bedrock)",
        enabled: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::BEDROCK_CLAUDE_OPUS_5,
        provider_id: seed_ids::BEDROCK_PROVIDER,
        model_id: "global.anthropic.claude-opus-5",
        display_name: "Claude Opus 5 (Bedrock)",
        enabled: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::BEDROCK_CLAUDE_SONNET_5,
        provider_id: seed_ids::BEDROCK_PROVIDER,
        model_id: "global.anthropic.claude-sonnet-5",
        display_name: "Claude Sonnet 5 (Bedrock)",
        enabled: false,
        is_favorite: false,
    },
    // LlmSim (simulated LLM for testing)
    SeedModel {
        id: seed_ids::LLMSIM_DEFAULT,
        provider_id: seed_ids::LLMSIM_PROVIDER,
        model_id: "llmsim-default",
        display_name: "LlmSim Default",
        enabled: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::LLMSIM_LATENCY,
        provider_id: seed_ids::LLMSIM_PROVIDER,
        model_id: "llmsim-latency",
        display_name: "LlmSim Latency",
        enabled: false,
        is_favorite: false,
    },
];
