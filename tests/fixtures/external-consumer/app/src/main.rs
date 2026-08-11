//! External-consumer acceptance fixture: an out-of-workspace application that
//! depends only on the public `everruns` Framework crate and runs offline
//! turns. Exercised by `scripts/test-external-consumer.sh`.
//!
//! The second turn installs capabilities from `external-capability-pack`, a
//! crate that depends on the neutral `everruns-capability` contract alone
//! (EVE-873): a code-defined `Definition` executes a scripted tool call, and
//! an `IntoCapability` value activates an open third-party reference — with
//! no core/host imports and no central enum edit anywhere.

use everruns::{Agent, LlmSimConfig, Model, ToolCall};
use external_capability_pack::{VendorSearch, math_pack};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .instructions("Return only the answer.")
        .model(Model::simulated("4"))
        .capability("session_file_system")
        .build()?;
    let session = agent.session();
    let context = session.inspect().await?;
    assert!(
        context.tools.iter().any(|tool| tool.name == "read_file"),
        "the default Framework facade must compose its filesystem integration"
    );
    let turn = session.run("What is 2 + 2?").await?;

    assert!(turn.success);
    assert_eq!(turn.response, "4");

    // External capability pack: code-defined capability + open reference.
    let sim = LlmSimConfig::fixed("4").with_tool_call_sequence(vec![
        vec![ToolCall {
            id: "call_add".into(),
            name: "external_add".into(),
            arguments: serde_json::json!({ "a": 2, "b": 2 }),
        }],
        vec![],
    ]);
    let agent = Agent::builder()
        .instructions("Use external_add for sums; answer with the digits only.")
        .model(Model::simulated_with_config(sim))
        .capability(math_pack())
        .capability(VendorSearch {
            index: "fixtures".into(),
        })
        .build()?;
    let session = agent.session();

    let context = session.inspect().await?;
    assert!(
        context
            .tools
            .iter()
            .any(|tool| tool.name == "external_add"),
        "external pack tool must register through the facade"
    );

    let turn = session.run("What is 2 + 2?").await?;
    assert!(turn.success);
    assert_eq!(turn.tool_calls, 1, "external_add executed once");
    assert_eq!(turn.response, "4");

    println!("{}", turn.response);
    Ok(())
}
