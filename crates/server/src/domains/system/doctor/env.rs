use crate::auth::config::{AuthConfig, AuthMode};
use crate::storage::blob_store::BlobStoreConfig;
use everruns_core::DeploymentGrade;
use everruns_core::config::{env_bool, env_opt_string, env_string};

/// Platform provider keys, by the name an operator sets.
const PROVIDER_KEYS: &[(&str, &str)] = &[
    ("DEFAULT_OPENAI_API_KEY", "OpenAI"),
    ("DEFAULT_ANTHROPIC_API_KEY", "Anthropic"),
    ("DEFAULT_GEMINI_API_KEY", "Gemini"),
    ("DEFAULT_AZURE_OPENAI_API_KEY", "Azure OpenAI"),
    ("DEFAULT_OPENROUTER_API_KEY", "OpenRouter"),
    ("DEFAULT_FIREWORKS_API_KEY", "Fireworks"),
    ("DEFAULT_META_API_KEY", "Meta"),
];

/// The configuration the checks judge, read once at startup.
///
/// Holds presence and lengths of secrets, never their values, except the
/// connection URLs the live probes need; those are never put in a report.
#[derive(Clone, Debug)]
pub struct DoctorEnv {
    pub grade: DeploymentGrade,
    pub auth_mode: AuthMode,
    pub base_url: String,
    pub jwt_secret_len: usize,
    /// `WORKER_GRPC_AUTH_TOKEN` length, `None` when unset.
    pub worker_token_len: Option<usize>,
    /// `DEV_MODE`: embedded database and an in-process worker.
    pub dev_mode: bool,
    pub utility_llm_configured: bool,
    pub nats_url: Option<String>,
    pub valkey_url: Option<String>,
    /// `STORAGE_BLOB_BACKEND` as set (`db` when unset).
    pub blob_backend: String,
    /// The S3 settings when the backend is `s3`, or why they do not parse.
    pub s3: Option<Result<BlobStoreConfig, String>>,
    pub metrics_enabled: bool,
    /// Whether `/metrics` is reachable at all: on `METRICS_ADDR`, or on the
    /// main listener with `METRICS_PUBLIC_ON_MAIN`.
    pub metrics_exposed: bool,
    pub otlp_endpoint: bool,
    pub otel_disabled: bool,
    pub rust_log: Option<String>,
    /// Display names of the providers with a platform key set.
    pub provider_keys: Vec<&'static str>,
}

impl DoctorEnv {
    pub fn from_env(auth: &AuthConfig, utility_llm_configured: bool) -> Self {
        let blob_backend = env_string("STORAGE_BLOB_BACKEND", "db");
        let s3 = (blob_backend == "s3")
            .then(|| BlobStoreConfig::from_env().map_err(|error| error.to_string()));
        Self {
            grade: DeploymentGrade::from_env(),
            auth_mode: auth.mode.clone(),
            base_url: auth.base_url.clone(),
            jwt_secret_len: auth.jwt.secret.len(),
            worker_token_len: env_opt_string("WORKER_GRPC_AUTH_TOKEN").map(|token| token.len()),
            dev_mode: env_bool("DEV_MODE", false),
            utility_llm_configured,
            nats_url: env_opt_string("NATS_URL"),
            valkey_url: env_opt_string("VALKEY_URL"),
            blob_backend,
            s3,
            metrics_enabled: env_bool("METRICS_ENABLED", true),
            metrics_exposed: env_opt_string("METRICS_ADDR").is_some()
                || env_bool("METRICS_PUBLIC_ON_MAIN", false),
            otlp_endpoint: env_opt_string("OTEL_EXPORTER_OTLP_ENDPOINT").is_some(),
            otel_disabled: env_bool("OTEL_SDK_DISABLED", false),
            rust_log: env_opt_string("RUST_LOG"),
            provider_keys: PROVIDER_KEYS
                .iter()
                .filter(|(var, _)| env_opt_string(var).is_some())
                .map(|(_, name)| *name)
                .collect(),
        }
    }

    /// Whether the install claims to be production, so unsafe settings are
    /// problems rather than warnings.
    pub fn strict(&self) -> bool {
        !self.grade.is_dev()
    }

    pub(super) fn grade_name(&self) -> String {
        self.grade.to_string()
    }

    pub(super) fn auth_mode_name(&self) -> String {
        self.auth_mode.as_str().to_string()
    }
}
