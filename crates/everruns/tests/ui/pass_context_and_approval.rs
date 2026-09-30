//! A leading call context and both approval forms expand and compile.
use everruns::approval::{ApprovalDecision, ToolApprover, async_trait};
use everruns::{Agent, Model, SessionId, ToolCall, ToolCallContext, ToolDefinition};

/// Context by value.
#[everruns::tool(needs_approval)]
async fn wipe(ctx: ToolCallContext) -> String {
    ctx.tool_call_id().to_string()
}

/// Context by reference, with a typed approval rule.
#[everruns::tool(needs_approval = |args: &RunSqlArgs| args.sql.contains("drop"))]
pub async fn run_sql(ctx: &everruns::ToolCallContext, sql: String) -> Result<String, String> {
    ctx.progress("running").await;
    Ok(sql)
}

fn never(_: &PingArgs) -> bool {
    false
}

/// A function path as the rule and no arguments.
#[everruns::tool(needs_approval = never)]
async fn ping() {}

struct Allow;

#[async_trait]
impl ToolApprover for Allow {
    async fn approve(&self, _: SessionId, _: &ToolCall, _: &ToolDefinition) -> ApprovalDecision {
        ApprovalDecision::Allow
    }
}

fn main() {
    let _args = RunSqlArgs { sql: "select 1".to_string() };
    let _agent = Agent::builder()
        .instructions("Use the tools.")
        .model(Model::simulated("ok"))
        .tool(wipe())
        .tool(run_sql())
        .tool(ping())
        .approver(Allow)
        .build()
        .expect("agent builds");
}
