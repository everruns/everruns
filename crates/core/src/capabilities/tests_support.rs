//! Fixtures shared by the test modules.

use super::*;
use crate::message::RuntimeMessage;
use crate::message_filter::{MessageFilter, MessageFilterProvider, MessageQuery};
use crate::tool_types::ToolCall;
use crate::tools::{Tool, ToolExecutionResult};
use crate::typed_id::SessionId;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

/// Test helper: dummy context with no file store
pub(crate) fn test_ctx() -> SystemPromptContext {
    SystemPromptContext::without_file_store(SessionId::new())
}

// -------------------------------------------------------------------------
// Local stand-ins for the fixture capabilities that moved to the
// `everruns-test-support` crate (EVE-875). The registry/apply/dependency
// mechanics tested here only need capabilities with these shapes: one
// that contributes nothing, one that contributes plain tools, and one
// that carries mounts plus a dependency.
// -------------------------------------------------------------------------

pub(crate) struct StubSubagentSpawnTool;

#[async_trait]
impl Tool for StubSubagentSpawnTool {
    fn name(&self) -> &str {
        "spawn_agent"
    }
    fn description(&self) -> &str {
        "stub subagent delegation"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }
    fn narrate(
        &self,
        tool_call: &ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(crate::tool_narration::narrate_subagent_spawn(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }
    async fn execute(&self, _arguments: serde_json::Value) -> crate::ToolExecutionResult {
        crate::ToolExecutionResult::success(serde_json::json!({}))
    }
}

pub(crate) fn spawn_agent_call(arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: "call-1".to_string(),
        name: "spawn_agent".to_string(),
        arguments,
    }
}

/// Contributes nothing: no tools, no prompt, no dependencies.
pub(crate) struct NoopFixture;

impl Capability for NoopFixture {
    fn id(&self) -> &str {
        "noop"
    }
    fn name(&self) -> &str {
        "No-Op"
    }
    fn description(&self) -> &str {
        "Contributes nothing."
    }
}

/// Declares one arbitrary feature to exercise core's neutral projection.
pub(crate) struct FeatureFixture;

impl Capability for FeatureFixture {
    fn id(&self) -> &str {
        "feature_fixture"
    }
    fn name(&self) -> &str {
        "Feature Fixture"
    }
    fn description(&self) -> &str {
        "Declares one test-only feature."
    }
    fn features(&self) -> Vec<&'static str> {
        vec!["fixture_feature"]
    }
}

pub(crate) struct FixtureTool(pub(crate) &'static str);

#[async_trait]
impl Tool for FixtureTool {
    fn name(&self) -> &str {
        self.0
    }
    fn description(&self) -> &str {
        "Fixture tool."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }
    async fn execute(&self, _arguments: serde_json::Value) -> ToolExecutionResult {
        ToolExecutionResult::success(serde_json::json!({ "ok": true }))
    }
}

pub(crate) struct BackgroundFixtureTool;

#[async_trait]
impl Tool for BackgroundFixtureTool {
    fn name(&self) -> &str {
        "bash"
    }
    fn description(&self) -> &str {
        "Fixture background-capable shell tool."
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    async fn execute(&self, _arguments: serde_json::Value) -> ToolExecutionResult {
        ToolExecutionResult::success(serde_json::json!({"ok": true}))
    }
    fn hints(&self) -> crate::tool_types::ToolHints {
        crate::tool_types::ToolHints {
            supports_background: Some(true),
            ..Default::default()
        }
    }
}

pub(crate) struct FileSystemFixture;

impl Capability for FileSystemFixture {
    fn id(&self) -> &str {
        "session_file_system"
    }
    fn name(&self) -> &str {
        "Fixture Filesystem"
    }
    fn description(&self) -> &str {
        "Fixture filesystem capability."
    }
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(FixtureTool("read_file")),
            Box::new(FixtureTool("write_file")),
        ]
    }
    fn features(&self) -> Vec<&'static str> {
        vec!["file_system"]
    }
}

/// Stands in for the product `session_storage` capability, which moved to
/// `everruns-platform` with the other service-backed families (EVE-886).
/// Feature computation is core's mechanism, so it is exercised here against
/// a fixture rather than a product implementation.
pub(crate) struct StorageFixture;

impl Capability for StorageFixture {
    fn id(&self) -> &str {
        "session_storage"
    }
    fn name(&self) -> &str {
        "Fixture Storage"
    }
    fn description(&self) -> &str {
        "Fixture session storage capability."
    }
    fn features(&self) -> Vec<&'static str> {
        vec!["secrets", "key_value"]
    }
}

pub(crate) struct BashFixture;

impl Capability for BashFixture {
    fn id(&self) -> &str {
        "bashkit_shell"
    }
    fn aliases(&self) -> Vec<&'static str> {
        vec!["virtual_bash"]
    }
    fn name(&self) -> &str {
        "Fixture Bash"
    }
    fn description(&self) -> &str {
        "Fixture shell capability."
    }
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(BackgroundFixtureTool)]
    }
    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_file_system"]
    }
    fn features(&self) -> Vec<&'static str> {
        vec!["file_system"]
    }
    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }
}

pub(crate) struct WebFetchFixture;

impl Capability for WebFetchFixture {
    fn id(&self) -> &str {
        "web_fetch"
    }
    fn name(&self) -> &str {
        "Fixture Web Fetch"
    }
    fn description(&self) -> &str {
        "Fixture web capability."
    }
    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }
}

/// Portable-policy-shaped stand-ins used only to exercise neutral core
/// collection mechanics after policy implementations moved out of core.
pub(crate) struct DynamicFactFixture;

impl Capability for DynamicFactFixture {
    fn id(&self) -> &str {
        "current_time"
    }
    fn name(&self) -> &str {
        "Dynamic Fact Fixture"
    }
    fn description(&self) -> &str {
        "Fixture with one dynamic fact and one tool."
    }
    fn icon(&self) -> Option<&str> {
        Some("clock")
    }
    fn category(&self) -> Option<&str> {
        Some("Core")
    }
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(FixtureTool("get_current_time"))]
    }
    fn facts(&self, _config: &serde_json::Value, _ctx: &FactsContext) -> Vec<Fact> {
        vec![Fact::dynamic("current_time", "fixture-now")]
    }
}

pub(crate) struct PromptToolFixture;

impl Capability for PromptToolFixture {
    fn id(&self) -> &str {
        "prompt_tool_fixture"
    }
    fn name(&self) -> &str {
        "Prompt Tool Fixture"
    }
    fn description(&self) -> &str {
        "Fixture with a static prompt and tool."
    }
    fn system_prompt_addition(&self) -> Option<&str> {
        Some("Task Management uses the write_todos tool.")
    }
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(FixtureTool("write_todos"))]
    }
}

pub(crate) struct SecondPromptFixture;

impl Capability for SecondPromptFixture {
    fn id(&self) -> &str {
        "second_prompt_fixture"
    }
    fn name(&self) -> &str {
        "Second Prompt Fixture"
    }
    fn description(&self) -> &str {
        "Fixture with a second static prompt."
    }
    fn system_prompt_addition(&self) -> Option<&str> {
        Some("A second capability prompt contribution.")
    }
}

pub(crate) struct DynamicPreviewFixture;

impl Capability for DynamicPreviewFixture {
    fn id(&self) -> &str {
        "agent_instructions"
    }
    fn name(&self) -> &str {
        "Dynamic Preview Fixture"
    }
    fn description(&self) -> &str {
        "Fixture whose runtime prompt is dynamic."
    }
    fn system_prompt_preview(&self) -> Option<String> {
        Some("Reads AGENTS.md dynamically.".to_string())
    }
}

/// Contributes four plain calculator-style tools and no prompt addition.
pub(crate) struct MathFixture;

impl Capability for MathFixture {
    fn id(&self) -> &str {
        "test_math"
    }
    fn name(&self) -> &str {
        "Test Math"
    }
    fn description(&self) -> &str {
        "Fixture: calculator tools."
    }
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(FixtureTool("add")),
            Box::new(FixtureTool("subtract")),
            Box::new(FixtureTool("multiply")),
            Box::new(FixtureTool("divide")),
        ]
    }
}

/// Contributes two plain tools.
pub(crate) struct WeatherFixture;

impl Capability for WeatherFixture {
    fn id(&self) -> &str {
        "test_weather"
    }
    fn name(&self) -> &str {
        "Test Weather"
    }
    fn description(&self) -> &str {
        "Fixture: weather tools."
    }
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(FixtureTool("get_weather")),
            Box::new(FixtureTool("get_forecast")),
        ]
    }
}

/// Carries a read-only mount, a prompt addition, a feature, and a
/// dependency on `session_file_system`.
pub(crate) struct SampleDataFixture;

impl Capability for SampleDataFixture {
    fn id(&self) -> &str {
        "sample_data"
    }
    fn name(&self) -> &str {
        "Sample Data"
    }
    fn description(&self) -> &str {
        "Fixture: mounted sample files."
    }
    fn system_prompt_addition(&self) -> Option<&str> {
        Some("Read-only sample files are mounted at `/samples`.")
    }
    fn mounts(&self) -> Vec<MountPoint> {
        let samples_dir = MountDirectoryBuilder::new()
            .file("users.json", "[]")
            .build();
        vec![MountPoint::readonly("/samples", samples_dir, self.id())]
    }
    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_file_system"]
    }
    fn features(&self) -> Vec<&'static str> {
        vec!["file_system"]
    }
}

/// Registry of local contribution fixtures.
pub(crate) fn fixture_registry() -> CapabilityRegistry {
    let mut registry = CapabilityRegistry::new();
    registry.register(NoopFixture);
    registry.register(FeatureFixture);
    registry.register(MathFixture);
    registry.register(WeatherFixture);
    registry.register(SampleDataFixture);
    registry.register(FileSystemFixture);
    registry.register(StorageFixture);
    registry.register(BashFixture);
    registry.register(WebFetchFixture);
    registry.register(DynamicFactFixture);
    registry.register(PromptToolFixture);
    registry.register(SecondPromptFixture);
    registry.register(DynamicPreviewFixture);
    registry
}

/// A host-defined capability carrying annotations core knows nothing about.
pub(crate) struct HostAnnotatedCapability;

#[async_trait]
impl Capability for HostAnnotatedCapability {
    fn id(&self) -> &str {
        "host_annotated"
    }
    fn name(&self) -> &str {
        "Host Annotated"
    }
    fn description(&self) -> &str {
        "Test capability with host-owned metadata."
    }
    fn metadata(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({"icon": "sparkles", "group": "host"}))
    }
}

pub(crate) fn blueprint_with_schema(config_schema: Option<serde_json::Value>) -> AgentBlueprint {
    AgentBlueprint {
        id: "test_blueprint",
        name: "Test Blueprint",
        description: "Blueprint for config validation tests",
        model: BlueprintModel::Inherit,
        system_prompt: "Test prompt",
        tools: vec![],
        max_turns: None,
        config_schema,
    }
}

/// Test capability that provides a message filter
pub(crate) struct FilterTestCapability {
    pub(crate) priority: i32,
}

impl Capability for FilterTestCapability {
    fn id(&self) -> &str {
        "filter_test"
    }
    fn name(&self) -> &str {
        "Filter Test"
    }
    fn description(&self) -> &str {
        "Test capability with message filter"
    }
    fn message_filter_provider(&self) -> Option<Arc<dyn MessageFilterProvider>> {
        Some(Arc::new(FilterTestProvider {
            priority: self.priority,
        }))
    }
}

pub(crate) struct FilterTestProvider {
    pub(crate) priority: i32,
}

impl MessageFilterProvider for FilterTestProvider {
    fn apply_filters(&self, query: &mut MessageQuery, config: &serde_json::Value) {
        // Add a search filter based on config
        if let Some(search) = config.get("search").and_then(|v| v.as_str()) {
            query
                .filters
                .push(MessageFilter::Search(search.to_string()));
        }
    }

    fn priority(&self) -> i32 {
        self.priority
    }
}

pub(crate) struct DelegatingFilterCap {
    pub(crate) id: &'static str,
    pub(crate) inner: std::sync::Arc<InnerFilterCap>,
}
pub(crate) struct InnerFilterCap;

impl Capability for InnerFilterCap {
    fn id(&self) -> &str {
        "inner_filter"
    }
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        panic!("fast-path collection must not instantiate tools")
    }
    fn system_prompt_addition(&self) -> Option<&str> {
        panic!("fast-path collection must not collect prompts")
    }
    fn name(&self) -> &str {
        "Inner Filter"
    }
    fn description(&self) -> &str {
        "inner"
    }
    fn message_filter_provider(&self) -> Option<std::sync::Arc<dyn MessageFilterProvider>> {
        Some(std::sync::Arc::new(SentinelFilter))
    }
}
pub(crate) struct SentinelFilter;
impl MessageFilterProvider for SentinelFilter {
    fn apply_filters(&self, query: &mut MessageQuery, config: &serde_json::Value) {
        query.limit = config["limit"].as_i64();
    }
}
impl Capability for DelegatingFilterCap {
    fn id(&self) -> &str {
        self.id
    }
    fn name(&self) -> &str {
        "Delegating Filter"
    }
    fn description(&self) -> &str {
        "delegating"
    }
    fn message_filter_provider(&self) -> Option<std::sync::Arc<dyn MessageFilterProvider>> {
        None // outer provides nothing
    }
    fn resolve_for_model(&self, _model: Option<&str>) -> Option<&dyn Capability> {
        Some(&*self.inner)
    }
}

pub(crate) struct DelegatingMvpCap {
    pub(crate) id: &'static str,
    pub(crate) inner: std::sync::Arc<InnerMvpCap>,
}
pub(crate) struct InnerMvpCap;

impl Capability for InnerMvpCap {
    fn id(&self) -> &str {
        "inner_mvp"
    }
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        panic!("fast-path collection must not instantiate tools")
    }
    fn system_prompt_addition(&self) -> Option<&str> {
        panic!("fast-path collection must not collect prompts")
    }
    fn name(&self) -> &str {
        "Inner MVP"
    }
    fn description(&self) -> &str {
        "inner"
    }
    fn model_view_provider(
        &self,
    ) -> Option<std::sync::Arc<dyn crate::capabilities::ModelViewProvider>> {
        // Retain the input and expose the forwarded config and context.
        struct AppendingMvp;
        impl crate::capabilities::ModelViewProvider for AppendingMvp {
            fn apply_model_view(
                &self,
                mut messages: Vec<RuntimeMessage>,
                config: &serde_json::Value,
                context: &ModelViewContext<'_>,
            ) -> Vec<RuntimeMessage> {
                messages.push(RuntimeMessage::user(format!(
                    "{}:{}",
                    config["suffix"].as_str().unwrap(),
                    context.session_id
                )));
                messages
            }
        }
        Some(std::sync::Arc::new(AppendingMvp))
    }
}
impl Capability for DelegatingMvpCap {
    fn id(&self) -> &str {
        self.id
    }
    fn name(&self) -> &str {
        "Delegating MVP"
    }
    fn description(&self) -> &str {
        "delegating"
    }
    fn model_view_provider(
        &self,
    ) -> Option<std::sync::Arc<dyn crate::capabilities::ModelViewProvider>> {
        None // outer provides nothing
    }
    fn resolve_for_model(&self, model: Option<&str>) -> Option<&dyn Capability> {
        (model == Some("selected-model")).then_some(&*self.inner as &dyn Capability)
    }
}

pub(crate) struct SkillContributingCapability;

impl Capability for SkillContributingCapability {
    fn id(&self) -> &str {
        "contributes_skills"
    }
    fn name(&self) -> &str {
        "Contributes Skills"
    }
    fn description(&self) -> &str {
        "Test capability that contributes skills."
    }
    fn contribute_skills(&self) -> Vec<SkillContribution> {
        vec![
            SkillContribution::new("alpha-skill", "Alpha skill desc", "# Alpha\nDo alpha.")
                .with_files(vec![(
                    "scripts/a.sh".to_string(),
                    "#!/bin/sh\necho a\n".to_string(),
                )]),
            SkillContribution::new("beta-skill", "Beta skill desc", "# Beta\nDo beta.")
                .with_user_invocable(false),
        ]
    }
}

pub(crate) fn skill_md_from_entries(entries: &HashMap<String, MountEntry>) -> &str {
    match &entries.get("SKILL.md").expect("SKILL.md missing").source {
        MountSource::InlineFile { content, .. } => content.as_str(),
        _ => panic!("Expected InlineFile for SKILL.md"),
    }
}

pub(crate) struct LocalizedCapability;

impl Capability for LocalizedCapability {
    fn id(&self) -> &str {
        "localized"
    }
    fn name(&self) -> &str {
        "Localized"
    }
    fn description(&self) -> &str {
        "English description"
    }
    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some("Controls things."),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk-UA",
                name: Some("Регіональна"),
                description: None,
                config_description: None,
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Локалізована"),
                description: Some("Український опис"),
                config_description: Some("Керує налаштуваннями."),
                config_overlay: None,
            },
        ]
    }
}

pub(crate) struct DependencyFixture {
    pub(crate) id: String,
    pub(crate) deps: Vec<&'static str>,
    pub(crate) features: Vec<&'static str>,
}
impl Capability for DependencyFixture {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        &self.id
    }
    fn description(&self) -> &str {
        "Dependency fixture"
    }
    fn dependencies(&self) -> Vec<&'static str> {
        self.deps.clone()
    }
    fn features(&self) -> Vec<&'static str> {
        self.features.clone()
    }
}
