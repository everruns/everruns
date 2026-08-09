//! Consumer-facing API test for the advanced capability SPI.
//!
//! Imports only `everruns`: no core/runtime registry, store, host, or tenancy
//! type is needed to define and install a multi-tool capability.

#![cfg(feature = "capabilities")]

use everruns::{Agent, BuildError, Model, capability};

#[derive(capability::Deserialize, capability::JsonSchema)]
#[serde(crate = "everruns::capability::serde")]
#[schemars(crate = "everruns::capability::schemars")]
struct SumInput {
    values: Vec<i64>,
}

#[derive(capability::Serialize, capability::JsonSchema)]
#[serde(crate = "everruns::capability::serde")]
#[schemars(crate = "everruns::capability::schemars")]
struct SumOutput {
    total: i64,
}

struct Sum;

#[capability::async_trait]
impl capability::Handler for Sum {
    type Input = SumInput;
    type Output = SumOutput;
    type Error = capability::Error;

    fn name(&self) -> &str {
        "sum_values"
    }

    fn description(&self) -> &str {
        "Sum a list of integer values."
    }

    async fn execute(
        &self,
        input: Self::Input,
        _context: capability::Context,
    ) -> Result<Self::Output, Self::Error> {
        Ok(SumOutput {
            total: input.values.into_iter().sum(),
        })
    }
}

#[test]
fn installs_capability_and_exposes_typed_protocol() {
    let arithmetic = capability::Definition::new(
        "custom_arithmetic",
        "Custom arithmetic",
        "Application-owned arithmetic tools.",
    )
    .tool(Sum);

    let schema = arithmetic.tools()[0].spec();
    assert_eq!(
        schema.input_schema()["properties"]["values"]["type"],
        "array"
    );
    assert_eq!(
        schema.output_schema()["properties"]["total"]["type"],
        "integer"
    );

    let agent = Agent::builder()
        .instructions("Use arithmetic tools when asked.")
        .model(Model::simulated("done"))
        .advanced_capability(arithmetic)
        .build();
    assert!(agent.is_ok(), "public advanced SPI should build: {agent:?}");
}

#[test]
fn rejects_invalid_descriptors_and_colliding_tools() {
    let empty = capability::Definition::new("empty", "Empty", "No tools.");
    let error = Agent::builder()
        .instructions("Do math.")
        .model(Model::simulated("done"))
        .advanced_capability(empty)
        .build()
        .expect_err("an advanced capability must define a tool");
    assert!(matches!(error, BuildError::InvalidCapability { .. }));

    let invalid = capability::Definition::new("bad id", "Bad", "Invalid id.").tool(Sum);
    let error = Agent::builder()
        .instructions("Do math.")
        .model(Model::simulated("done"))
        .advanced_capability(invalid)
        .build()
        .expect_err("invalid capability id must fail");
    assert!(matches!(error, BuildError::InvalidCapability { .. }));

    let first = capability::Definition::new("first", "First", "First tool.").tool(Sum);
    let second = capability::Definition::new("second", "Second", "Second tool.").tool(Sum);
    let error = Agent::builder()
        .instructions("Do math.")
        .model(Model::simulated("done"))
        .advanced_capability(first)
        .advanced_capability(second)
        .build()
        .expect_err("tool names must be unique across capabilities");
    assert_eq!(
        error,
        BuildError::DuplicateTool {
            name: "sum_values".into(),
        }
    );
}
