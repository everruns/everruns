//! External capability package: implements `IntoCapability` and a
//! code-defined capability against the neutral `everruns-capability` contract
//! only — no `everruns`, `everruns-core`, `everruns-host`, or Tokio imports.

use everruns_capability::definition::{
    self as capability, Context, Definition, Error, Handler, Hints,
};
use everruns_capability::{CapabilityRef, CapabilitySpec, IntoCapability};

/// A dynamic, database-driven style capability reference. The referenced
/// implementation owns the inner config schema; only the id grammar and JSON
/// object boundary are validated centrally.
pub struct VendorSearch {
    pub index: String,
}

impl IntoCapability for VendorSearch {
    fn into_capability(self) -> CapabilitySpec {
        CapabilityRef::new("vendor.search")
            .config(capability::serde_json::json!({ "index": self.index }))
            .into()
    }
}

#[derive(capability::Deserialize, capability::JsonSchema)]
#[serde(crate = "everruns_capability::serde")]
#[schemars(crate = "everruns_capability::schemars")]
pub struct AddInput {
    pub a: i64,
    pub b: i64,
}

#[derive(capability::Serialize, capability::JsonSchema)]
#[serde(crate = "everruns_capability::serde")]
#[schemars(crate = "everruns_capability::schemars")]
pub struct AddOutput {
    pub sum: i64,
}

struct Add;

#[capability::async_trait]
impl Handler for Add {
    type Input = AddInput;
    type Output = AddOutput;
    type Error = Error;

    fn name(&self) -> &str {
        "external_add"
    }

    fn description(&self) -> &str {
        "Add two integers using the external capability pack."
    }

    fn hints(&self) -> Hints {
        Hints::default().readonly(true).idempotent(true)
    }

    async fn execute(&self, input: Self::Input, context: Context) -> Result<Self::Output, Error> {
        context.progress("adding").await;
        Ok(AddOutput {
            sum: input.a + input.b,
        })
    }
}

/// A reusable code-defined capability exported by this package.
pub fn math_pack() -> Definition {
    Definition::new(
        "external_math",
        "External Math",
        "Typed math tools from an out-of-workspace capability crate.",
    )
    .instructions("Use external_add for exact integer sums.")
    .tool(Add)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definition_is_valid_and_exposes_schemas() {
        let pack = math_pack();
        pack.validate().expect("valid definition");
        let spec = pack.tools()[0].spec();
        assert_eq!(spec.name(), "external_add");
        assert_eq!(spec.input_schema()["type"], "object");
        assert_eq!(spec.output_schema()["properties"]["sum"]["type"], "integer");
        assert_eq!(spec.hints().readonly, Some(true));
    }

    #[test]
    fn dynamic_reference_serializes_to_the_persisted_shape() {
        let spec = VendorSearch {
            index: "prod".into(),
        }
        .into_capability();
        let value = capability::serde_json::to_value(spec.capability_ref()).unwrap();
        assert_eq!(
            value,
            capability::serde_json::json!({"ref": "vendor.search", "config": {"index": "prod"}})
        );
        // Config payloads never appear in Debug output.
        let debug = format!("{:?}", spec.capability_ref());
        assert!(!debug.contains("prod"));
    }
}
