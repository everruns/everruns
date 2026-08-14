//! Catalog-backed platform management capability.

use async_trait::async_trait;
use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, RiskLevel,
};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::{tool_context::ToolContext, tool_context::ToolContextService};
use everruns_provider::tool_types::{DeferrablePolicy, ToolHints};
use serde_json::{Value, json};

pub const PLATFORM_CAPABILITY_ID: &str = "platform";

pub const DISCOVER_DESCRIPTION: &str = "Search the Everruns operation catalog to find commands, not resource instances. Resource-oriented phrases rank relevant list/get operations, but zero matches never prove that a plugin, capability, agent, or other resource is absent. Inspect current resource state by running authoritative read operations with query. Multi-match searches return a compact name list; call discover once more with the exact command name to get its schema and copyable bash_usage. Do not use all: true for a task-specific lookup.";
pub const QUERY_DESCRIPTION: &str = "Execute a bash script in an environment where only read-only Everruns API operations are available as built-in commands. Supports pipes, variables, loops, conditionals, jq, and direct access to safe builtins. Mutating commands are intentionally unavailable.";
pub const EXECUTE_DESCRIPTION: &str = "Execute a bash script in an environment where every Everruns API operation is a built-in command, including operations with side effects. Supports pipes, variables, loops, conditionals, jq, and direct access to the full builtin set. Prefer query for read-only inspection; use execute when you need create/update/delete or other mutating operations.";

const SYSTEM_PROMPT: &str = r#"Use `discover` only to find an Everruns operation or unknown schema. It searches operation metadata, not resource instances: a zero-match result never proves that a plugin, capability, agent, harness, MCP server, connection, model, knowledge index, schedule, or other entity is absent. For a resource question, discover the relevant authoritative list/get operation once if needed, then run it with `query`; do not repeat discovery or request a giant inventory. Before mutations, use one bounded query script to resolve names, status, references, attachments, and user-scoped connection state. Treat installed, active/available, attached, and connected as separate facts and report only fields returned by authoritative reads. Multi-match search results are intentionally compact; pick the operation name and discover that exact name once to get `bash_usage` and `output_fields`. Do not use `all: true` for a task-specific lookup and do not rediscover a single-match result. Invoke commands with exactly the flags in `bash_usage`, and use `output_fields` to select jq paths instead of guessing resource fields. Platform builtins do not implement `--help`; do not probe them with `--help`, shell inventory commands, or saved-output searches. Filesystem redirection is disabled: never write intermediate output to `/tmp` or another file; keep JSON in shell variables and pipe it directly to `jq`. Pass array and object flags as JSON text. For dependent mutations, capture JSON output and use `jq` to feed returned IDs or capability references to later commands in the same `execute` script. `create_mcp_server` returns `capability_ref`, which is accepted in `create_agent --capabilities`; there is no separate MCP attachment operation. Use `query` for read-only inspection and validation. Use `execute` only for user-requested mutations, then validate the resulting state with `query`. Do not guess operation names or flags. Platform commands are already scoped to the current organization."#;

pub struct PlatformCapability;

impl Capability for PlatformCapability {
    fn id(&self) -> &str {
        PLATFORM_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Platform"
    }

    fn description(&self) -> &str {
        "Discover and execute the authorized Everruns command catalog."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Платформа",
            "Пошук і виконання дозволених команд Everruns.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }

    fn icon(&self) -> Option<&str> {
        Some("terminal-square")
    }

    fn category(&self) -> Option<&str> {
        Some("Platform")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(PlatformCommandTool::discover()),
            Box::new(PlatformCommandTool::query()),
            Box::new(PlatformCommandTool::execute()),
        ]
    }
}

#[derive(Debug, Clone, Copy)]
enum PlatformCommandOperation {
    Discover,
    Query,
    Execute,
}

struct PlatformCommandTool {
    operation: PlatformCommandOperation,
}

impl PlatformCommandTool {
    const fn discover() -> Self {
        Self {
            operation: PlatformCommandOperation::Discover,
        }
    }

    const fn query() -> Self {
        Self {
            operation: PlatformCommandOperation::Query,
        }
    }

    const fn execute() -> Self {
        Self {
            operation: PlatformCommandOperation::Execute,
        }
    }
}

#[async_trait]
impl Tool for PlatformCommandTool {
    fn narrate(
        &self,
        _tool_call: &everruns_provider::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        use everruns_core::tool_narration::ToolNarrationPhase;

        let ukrainian = locale.is_some_and(|value| value.to_ascii_lowercase().starts_with("uk"));
        let text = match (self.operation, phase, ukrainian) {
            (PlatformCommandOperation::Discover, ToolNarrationPhase::Completed, false) => {
                "Discovered operations"
            }
            (PlatformCommandOperation::Discover, ToolNarrationPhase::Failed, false) => {
                "Could not discover operations"
            }
            (PlatformCommandOperation::Discover, _, false) => "Discovering operations",
            (PlatformCommandOperation::Query, ToolNarrationPhase::Completed, false) => {
                "Queried platform"
            }
            (PlatformCommandOperation::Query, ToolNarrationPhase::Failed, false) => {
                "Could not query platform"
            }
            (PlatformCommandOperation::Query, _, false) => "Querying platform",
            (PlatformCommandOperation::Execute, ToolNarrationPhase::Completed, false) => {
                "Executed platform commands"
            }
            (PlatformCommandOperation::Execute, ToolNarrationPhase::Failed, false) => {
                "Could not execute platform commands"
            }
            (PlatformCommandOperation::Execute, _, false) => "Executing platform commands",
            (PlatformCommandOperation::Discover, ToolNarrationPhase::Completed, true) => {
                "Знайдено операції"
            }
            (PlatformCommandOperation::Discover, ToolNarrationPhase::Failed, true) => {
                "Не вдалося знайти операції"
            }
            (PlatformCommandOperation::Discover, _, true) => "Шукаю операції",
            (PlatformCommandOperation::Query, ToolNarrationPhase::Completed, true) => {
                "Опитано платформу"
            }
            (PlatformCommandOperation::Query, ToolNarrationPhase::Failed, true) => {
                "Не вдалося опитати платформу"
            }
            (PlatformCommandOperation::Query, _, true) => "Опитую платформу",
            (PlatformCommandOperation::Execute, ToolNarrationPhase::Completed, true) => {
                "Виконано команди платформи"
            }
            (PlatformCommandOperation::Execute, ToolNarrationPhase::Failed, true) => {
                "Не вдалося виконати команди платформи"
            }
            (PlatformCommandOperation::Execute, _, true) => "Виконую команди платформи",
        };
        Some(text.to_string())
    }

    fn name(&self) -> &str {
        match self.operation {
            PlatformCommandOperation::Discover => "discover",
            PlatformCommandOperation::Query => "query",
            PlatformCommandOperation::Execute => "execute",
        }
    }

    fn display_name(&self) -> Option<&str> {
        Some(match self.operation {
            PlatformCommandOperation::Discover => "Discover Operations",
            PlatformCommandOperation::Query => "Query Commands",
            PlatformCommandOperation::Execute => "Execute Commands",
        })
    }

    fn description(&self) -> &str {
        match self.operation {
            PlatformCommandOperation::Discover => DISCOVER_DESCRIPTION,
            PlatformCommandOperation::Query => QUERY_DESCRIPTION,
            PlatformCommandOperation::Execute => EXECUTE_DESCRIPTION,
        }
    }

    fn parameters_schema(&self) -> Value {
        match self.operation {
            PlatformCommandOperation::Discover => discover_input_schema(),
            PlatformCommandOperation::Query => query_input_schema(),
            PlatformCommandOperation::Execute => execute_input_schema(),
        }
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Platform command context is required")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        if arguments.get("organization_id").is_some() {
            return ToolExecutionResult::tool_error(
                "organization_id is not accepted; Platform commands use the current session organization",
            );
        }
        let Some(store) = context.extension::<crate::platform_store::PlatformStoreExt>() else {
            return ToolExecutionResult::tool_error("Platform command service is unavailable");
        };
        let store = &store.0;
        let result = match self.operation {
            PlatformCommandOperation::Discover => store.platform_discover(arguments).await,
            PlatformCommandOperation::Query => store.platform_query(arguments).await,
            PlatformCommandOperation::Execute => store.platform_execute(arguments).await,
        };
        match result {
            Ok(output) => ToolExecutionResult::success(output),
            Err(error) => ToolExecutionResult::tool_error(error.to_string()),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[]
    }

    fn hints(&self) -> ToolHints {
        match self.operation {
            PlatformCommandOperation::Discover | PlatformCommandOperation::Query => ToolHints {
                readonly: Some(true),
                destructive: Some(false),
                idempotent: Some(true),
                open_world: Some(false),
                ..Default::default()
            },
            PlatformCommandOperation::Execute => ToolHints {
                readonly: Some(false),
                destructive: Some(true),
                idempotent: Some(false),
                open_world: Some(true),
                ..Default::default()
            },
        }
    }

    fn deferrable_policy(&self) -> DeferrablePolicy {
        DeferrablePolicy::Never
    }
}

pub fn discover_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "query": {
                "type": "string",
                "description": "Search operation metadata. Resource names may help rank list/get operations, but matches and misses do not report resource existence."
            },
            "all": {
                "type": "boolean",
                "description": "List all available operations grouped by category. When true, query is ignored."
            },
            "include_schemas": {
                "type": "boolean",
                "description": "For an exact-name lookup, include authoritative bash_usage/output_fields and bounded expanded schemas. Defaults to true for search and false for all. Omit this field normally; never repeat an exact lookup with a different value."
            }
        }
    })
}

pub fn query_input_schema() -> Value {
    script_input_schema(
        "Bash script to execute. Only read-only API operations are available as built-in commands.",
    )
}

pub fn execute_input_schema() -> Value {
    script_input_schema(
        "Bash script to execute. API operations are available as built-in commands.",
    )
}

fn script_input_schema(commands_description: &str) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "commands": {
                "type": "string",
                "description": commands_description,
                "minLength": 1
            },
            "timeout_ms": {
                "type": "integer",
                "description": "Execution timeout in milliseconds (default: 30000, max: 60000).",
                "minimum": 1,
                "maximum": 60000
            }
        },
        "required": ["commands"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform_store::tests::MockPlatformStore;
    use everruns_provider::typed_id::SessionId;
    use std::sync::Arc;

    #[test]
    fn schemas_do_not_allow_organization_override() {
        assert!(
            discover_input_schema()["properties"]
                .get("organization_id")
                .is_none()
        );
        assert!(
            query_input_schema()["properties"]
                .get("organization_id")
                .is_none()
        );
        assert!(
            execute_input_schema()["properties"]
                .get("organization_id")
                .is_none()
        );
    }

    #[test]
    fn tools_own_phase_aware_human_narration() {
        use everruns_core::tool_narration::{ToolNarrationContext, ToolNarrationPhase};
        use everruns_provider::tool_types::ToolCall;

        let cases = [
            (
                PlatformCommandTool::discover(),
                "discover",
                "Discovering operations",
                "Discovered operations",
            ),
            (
                PlatformCommandTool::query(),
                "query",
                "Querying platform",
                "Queried platform",
            ),
            (
                PlatformCommandTool::execute(),
                "execute",
                "Executing platform commands",
                "Executed platform commands",
            ),
        ];

        for (tool, name, started, completed) in cases {
            let call = ToolCall {
                id: format!("call_{name}"),
                name: name.to_string(),
                arguments: json!({}),
            };
            assert_eq!(
                tool.narrate(
                    &call,
                    ToolNarrationPhase::Started,
                    None,
                    ToolNarrationContext::new(None),
                )
                .as_deref(),
                Some(started)
            );
            assert_eq!(
                tool.narrate(
                    &call,
                    ToolNarrationPhase::Completed,
                    None,
                    ToolNarrationContext::new(None),
                )
                .as_deref(),
                Some(completed)
            );
        }
    }

    #[test]
    fn discovery_contract_distinguishes_operations_from_resources() {
        assert!(DISCOVER_DESCRIPTION.contains("not resource instances"));
        assert!(DISCOVER_DESCRIPTION.contains("zero matches never prove"));
        assert!(SYSTEM_PROMPT.contains("one bounded query script"));
        assert!(SYSTEM_PROMPT.contains("installed, active/available, attached, and connected"));
    }

    #[tokio::test]
    async fn tools_dispatch_to_platform_store() {
        let store = Arc::new(MockPlatformStore::new());
        let context = ToolContext::new(SessionId::new()).with_extension(Arc::new(
            crate::platform_store::PlatformStoreExt(
                store as Arc<dyn crate::platform_store::PlatformStore>,
            ),
        ));
        let result = PlatformCommandTool::query()
            .execute_with_context(json!({ "commands": "list_models" }), &context)
            .await;
        assert!(result.is_success());
    }

    #[tokio::test]
    async fn organization_override_is_rejected_even_if_injected() {
        let store = Arc::new(MockPlatformStore::new());
        let context = ToolContext::new(SessionId::new()).with_extension(Arc::new(
            crate::platform_store::PlatformStoreExt(
                store as Arc<dyn crate::platform_store::PlatformStore>,
            ),
        ));
        let result = PlatformCommandTool::query()
            .execute_with_context(
                json!({ "commands": "list_models", "organization_id": "org_other" }),
                &context,
            )
            .await;
        assert!(matches!(result, ToolExecutionResult::ToolError(_)));
    }
}
