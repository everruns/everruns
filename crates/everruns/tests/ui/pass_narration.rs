//! The narration hooks from docs/framework/advanced-capabilities.md compile:
//! a `Handler::narrate` override and the `#[everruns::tool(narrate = …)]`
//! option, both built from the public `everruns::narration` helpers.
use everruns::{Agent, Model, capability, narration};

#[derive(capability::Deserialize, capability::JsonSchema)]
#[serde(crate = "everruns::capability::serde")]
#[schemars(crate = "everruns::capability::schemars")]
struct LookupInput {
    id: String,
}

struct Lookup;

#[capability::async_trait]
impl capability::Handler for Lookup {
    type Input = LookupInput;
    type Output = String;
    type Error = capability::Error;

    fn name(&self) -> &str {
        "lookup_record"
    }

    fn description(&self) -> &str {
        "Look up one record by exact id."
    }

    fn narrate(&self, call: &capability::ToolNarration<'_>) -> Option<String> {
        narration::safe_arg_str(call.arguments(), &["id"])?;
        Some(narration::narrate_labeled_action(
            call.arguments(),
            call.phase(),
            call.locale(),
            ("Looking up record", "Looked up record", "Could not look up record"),
            ("Шукаю запис", "Знайшов запис", "Не вдалося знайти запис"),
            &["id"],
        ))
    }

    async fn execute(
        &self,
        input: Self::Input,
        _context: capability::Context,
    ) -> Result<Self::Output, Self::Error> {
        Ok(input.id)
    }
}

/// Convert Celsius to Fahrenheit.
#[everruns::tool(narrate = |args: &FahrenheitArgs, _call: &everruns::ToolNarration<'_>| {
    Some(format!("Converting {} °C", narration::truncate(&args.celsius.to_string(), 16)))
})]
async fn fahrenheit(celsius: f64) -> f64 {
    celsius * 1.8 + 32.0
}

fn by_phase(args: &LabelArgs, call: &everruns::ToolNarration<'_>) -> Option<String> {
    match call.phase() {
        everruns::ToolNarrationPhase::Completed => Some(format!("Labeled {}", args.name)),
        _ => None,
    }
}

/// A function path as the narrator.
#[everruns::tool(narrate = by_phase)]
async fn label(name: String) -> String {
    name
}

fn main() {
    let records = capability::Definition::new("records", "Records", "Record lookup.").tool(Lookup);
    let _agent = Agent::builder()
        .instructions("Use the tools.")
        .model(Model::simulated("done"))
        .capability(records)
        .tool(fahrenheit())
        .tool(label())
        .build()
        .expect("valid agent");
}
