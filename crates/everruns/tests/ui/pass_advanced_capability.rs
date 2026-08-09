use everruns::{Agent, Model, capability};

#[derive(capability::Deserialize, capability::JsonSchema)]
#[serde(crate = "everruns::capability::serde")]
#[schemars(crate = "everruns::capability::schemars")]
struct Input {
    value: u64,
}

#[derive(capability::Serialize, capability::JsonSchema)]
#[serde(crate = "everruns::capability::serde")]
#[schemars(crate = "everruns::capability::schemars")]
struct Output {
    doubled: u64,
}

struct Double;

#[capability::async_trait]
impl capability::Handler for Double {
    type Input = Input;
    type Output = Output;
    type Error = capability::Error;

    fn name(&self) -> &str { "double" }
    fn description(&self) -> &str { "Double an integer." }

    async fn execute(
        &self,
        input: Self::Input,
        _context: capability::Context,
    ) -> Result<Self::Output, Self::Error> {
        Ok(Output { doubled: input.value * 2 })
    }
}

fn main() {
    let capability = capability::Definition::new("math", "Math", "Typed math.").tool(Double);
    let _agent = Agent::builder()
        .instructions("Do math.")
        .model(Model::simulated("done"))
        .capability(capability)
        .build()
        .unwrap();
}
