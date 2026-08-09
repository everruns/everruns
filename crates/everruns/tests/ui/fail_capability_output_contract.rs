use everruns::capability;

#[derive(capability::Deserialize, capability::JsonSchema)]
#[serde(crate = "everruns::capability::serde")]
#[schemars(crate = "everruns::capability::schemars")]
struct Input;

struct NotSerializableOrSchema;
struct Broken;

#[capability::async_trait]
impl capability::Handler for Broken {
    type Input = Input;
    type Output = NotSerializableOrSchema;
    type Error = capability::Error;

    fn name(&self) -> &str { "broken" }
    fn description(&self) -> &str { "Does not satisfy the output contract." }

    async fn execute(
        &self,
        _input: Self::Input,
        _context: capability::Context,
    ) -> Result<Self::Output, Self::Error> {
        Ok(NotSerializableOrSchema)
    }
}

fn main() {}
