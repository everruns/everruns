// Harness examples: list and adopt them as commands.
//
// Decision: examples live in code (`crate::harnesses::examples`), not in the
// database, and are identified by name. Coding (the Yolop-based harness) and
// Data Analyst are examples, not built-ins: an org that wants one adopts it.
// Decision: listing and adopting are commands, so the CLI and MCP reach the
// same catalogue the console's Harnesses → Examples page does. Before this an
// agent working over MCP saw only the org's harnesses, concluded Coding did
// not exist and fell back to Sandbox Worker. `GET /v1/platform/harness-examples` and
// `POST /v1/platform/harnesses/import?from-example={name}` delegate here; there is one
// filter (dev-only, registered capabilities) and one adoption path.
// Decision: the deployment grade is read from the environment, as the REST
// state does, and the capability registry is the one `CapabilityService` was
// built with (same as `domains::agents::examples`).

use super::types::CreateHarnessRequest;
use super::{CheckHarnessName, CreateHarness, HARNESS_MANAGE, HARNESS_VIEW};
use crate::domains::common::*;
use crate::domains::harnesses::record::Harness;
use crate::harnesses::{HarnessExampleDef, find_harness_example, harness_examples};
use everruns_core::DeploymentGrade;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A read-only harness example defined in code, adoptable as an org harness.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct HarnessExample {
    /// Locale-keyed display names and descriptions.
    pub localizations: everruns_contracts::runtime::localization::EntityLocalizations,
    /// Unique slug (e.g. `coding`).
    #[schema(example = "coding")]
    pub name: String,
    /// Human-readable display name (e.g. `Coding`).
    #[schema(example = "Coding")]
    pub display_name: String,
    /// Short description.
    pub description: String,
    /// Display glyph name rendered by the UI (e.g. `bar-chart`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Tags for categorization.
    pub tags: Vec<String>,
    /// Name of the parent harness this example inherits from when adopted
    /// (e.g. `sandbox-worker`). Resolved per org at adoption; no UUIDs are
    /// hardcoded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_name: Option<String>,
    /// Capabilities the example will assign with their per-harness config.
    #[schema(value_type = Vec<crate::records::CapabilityRefSchema>)]
    pub capabilities: Vec<everruns_contracts::CapabilityRef>,
    /// Whether this example is only available when experimental features are on.
    pub dev_only: bool,
}

fn capability_refs(ex: &HarnessExampleDef) -> Vec<everruns_contracts::CapabilityRef> {
    ex.definition
        .capabilities
        .iter()
        .map(|cap| {
            everruns_contracts::CapabilityRef::with_config(
                cap.typed_id().clone(),
                cap.config_value().clone(),
            )
        })
        .collect()
}

fn to_example(ex: &HarnessExampleDef) -> HarnessExample {
    HarnessExample {
        localizations: ex.definition.localizations.clone(),
        name: ex.definition.name.clone(),
        display_name: ex.definition.display_name.clone(),
        description: ex.definition.description.clone(),
        icon: ex.definition.icon.clone(),
        tags: ex.definition.tags.clone(),
        parent_name: ex.definition.parent_name.clone(),
        capabilities: capability_refs(ex),
        dev_only: ex.dev_only,
    }
}

fn offered(
    ex: &HarnessExampleDef,
    include_dev: bool,
    has_capability: &dyn Fn(&str) -> bool,
) -> bool {
    (include_dev || !ex.dev_only)
        && ex
            .definition
            .capabilities
            .iter()
            .all(|cap| has_capability(cap.capability_id()))
}

/// The harness examples a deployment offers, in catalogue order.
///
/// An example is offered when it is not dev-only (or `include_dev` is set)
/// and every capability it assigns is registered on this deployment. The
/// REST list, the CLI and MCP all read this one filter.
pub fn offered_harness_examples(
    include_dev: bool,
    has_capability: &dyn Fn(&str) -> bool,
) -> Vec<HarnessExample> {
    harness_examples()
        .iter()
        .filter(|ex| offered(ex, include_dev, has_capability))
        .map(to_example)
        .collect()
}

fn include_dev() -> bool {
    DeploymentGrade::from_env().experimental_features_enabled()
}

// ============================================================================
// ListHarnessExamples
// ============================================================================

/// List the harness examples this deployment offers.
#[derive(Debug, Default, Deserialize, Serialize, ToSchema)]
#[serde(default)]
pub struct ListHarnessExamples {}

#[command(
    name = "list_harness_examples",
    category = "harnesses",
    description = "List adoptable harness examples (ready-made harnesses such as coding, the Yolop-based coding harness, and data-analyst): name, description, parent harness, capabilities. They are not in list_harnesses until adopted; adopt one by name with adopt_harness_example before building a harness from scratch.",
    method = "GET",
    path = "/v1/platform/harness-examples",
    policy = HARNESS_VIEW,
    cli = CliRoute::new(&["harnesses", "examples"], "list").with_examples(&[CliExample::new("Find a ready-made harness (such as coding) before writing one by hand", "everruns harnesses examples list")]),
)]
impl Command for ListHarnessExamples {
    type Output = Vec<HarnessExample>;

    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<HarnessExample>())
    }

    fn output_shape() -> &'static str {
        "array"
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<HarnessExample>, CommandError> {
        let registry = ctx.capability_service.registry();
        Ok(offered_harness_examples(include_dev(), &|id| {
            registry.has(id)
        }))
    }
}

// ============================================================================
// AdoptHarnessExample
// ============================================================================

/// Create an org harness from a harness example.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct AdoptHarnessExample {
    /// Example name from list_harness_examples, such as `coding`.
    pub name: String,
}

#[command(
    name = "adopt_harness_example",
    category = "harnesses",
    description = "Create an org harness from a harness example by name (see list_harness_examples), e.g. coding for the Yolop-based coding harness. The harness is named after the example, with a short suffix when that name is taken, and inherits from the example's parent harness. Agents then select it by that name.",
    method = "POST",
    path = "/v1/platform/harnesses/import",
    policy = HARNESS_MANAGE,
    positional = "name",
    cli = CliRoute::new(&["harnesses", "examples"], "adopt").with_args(&[CliArg::new("name").at(1)]).with_examples(&[CliExample::new("Add the Coding harness to the org so agents can use it", "everruns harnesses examples adopt coding --reason 'Coding agents need the Coding harness'")]),
)]
impl Command for AdoptHarnessExample {
    type Output = Harness;

    async fn execute(self, ctx: &Ctx) -> Result<Harness, CommandError> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(CommandError::bad_request(
                "Example name must not be empty; see list_harness_examples",
            ));
        }
        let not_found = || CommandError::not_found(&format!("harness example '{name}'"));
        let example = find_harness_example(name).ok_or_else(not_found)?;
        // A dev-only example does not exist outside experimental deployments.
        if example.dev_only && !include_dev() {
            return Err(not_found());
        }
        let registry = ctx.capability_service.registry();
        let missing: Vec<&str> = example
            .definition
            .capabilities
            .iter()
            .map(|c| c.capability_id())
            .filter(|id| !registry.has(id))
            .collect();
        if !missing.is_empty() {
            return Err(CommandError::bad_request(format!(
                "Example requires unregistered capabilities: {missing:?}"
            )));
        }

        // Examples inherit by name, resolved in this org; never a fixed UUID.
        let parent_harness_id = match example.definition.parent_name.as_deref() {
            Some(parent) => Some(
                ctx.db
                    .get_harness_by_name(ctx.org_id(), parent)
                    .await?
                    .ok_or_else(|| {
                        CommandError::internal(anyhow::anyhow!(
                            "Required parent harness `{parent}` is not provisioned for this org"
                        ))
                    })?
                    .id,
            ),
            None => None,
        };

        let req = CreateHarnessRequest {
            localizations: example.definition.localizations.clone(),
            name: unique_name(ctx, &example.definition.name).await?,
            display_name: Some(example.definition.display_name.clone()),
            description: Some(example.definition.description.clone()),
            intro_markdown: None,
            short_description: None,
            starters: Vec::new(),
            system_prompt: Some(example.definition.system_prompt.clone()),
            parent_harness_id,
            default_model_id: None,
            tags: example.definition.tags.clone(),
            capabilities: capability_refs(&example),
            initial_files: vec![],
            mcp_servers: Default::default(),
            network_access: None,
            embedder_metadata: Default::default(),
        };

        // Composed under this command's own policy (the same HARNESS_MANAGE)
        // and history entry; CreateHarness still validates capabilities and
        // enforces the per-org harness cap.
        CreateHarness(req).execute(ctx).await
    }
}

/// The example's name, or a suffixed variant when it is taken. Several tries
/// keep a one-shot collision (or a concurrent adoption) from failing.
async fn unique_name(ctx: &Ctx, base: &str) -> Result<String, CommandError> {
    use rand::RngExt;

    for attempt in 0..10 {
        let candidate = if attempt == 0 {
            base.to_string()
        } else {
            let suffix: String = rand::rng()
                .sample_iter(&rand::distr::Alphanumeric)
                .take(5)
                .map(|c| (c as char).to_ascii_lowercase())
                .collect();
            format!("{base}-{suffix}")
        };
        let availability = CheckHarnessName {
            name: candidate.clone(),
            exclude_id: None,
        }
        .execute(ctx)
        .await?;
        if availability.available {
            return Ok(candidate);
        }
    }
    Err(CommandError::conflict(format!(
        "No free harness name derived from '{base}'"
    )))
}

#[cfg(test)]
mod tests;
