use everruns_scale::{PortableAgent, RegistrationPath};

fn main() -> Result<(), everruns_scale::PortableAgentError> {
    let definition = PortableAgent::builder(
        "Answer weather questions concisely.",
        "gpt-5-mini",
        RegistrationPath::new("providers/openai/default")?,
        RegistrationPath::new("workspaces/project/v1")?,
    )
    .name("weather")
    .tool(RegistrationPath::new("tools/weather/v1")?)
    .hook(RegistrationPath::new("hooks/audit/v1")?)
    .build()?;

    println!("{}", serde_json::to_string_pretty(&definition).unwrap());
    Ok(())
}
