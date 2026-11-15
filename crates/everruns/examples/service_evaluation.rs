//! Real-agent evaluations and portable traces.
//! cargo run -p everruns --features openai,evaluation --example service_evaluation
use everruns::{
    Agent, Engine, Model, OpenAI,
    evaluation::{Case, Evaluator, Scorer},
    trace::Trace,
};
use std::time::Duration;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = OpenAI::from_env()?;
    let agent = Agent::builder()
        .instructions("Answer arithmetic with just the resulting integer.")
        .model(Model::new("gpt-6-luna", provider))
        .build()?;
    let evaluator = Evaluator::new(vec![
        Scorer::Regex {
            pattern: "^42[.!]?$".into(),
            weight: 1.0,
        },
        Scorer::TurnsWithin {
            max: 1,
            weight: 1.0,
        },
    ])?;
    let engine = Engine::new();
    let results = evaluator
        .run(
            &engine,
            agent,
            vec![Case {
                name: "arithmetic".into(),
                inputs: vec!["What is 6 times 7?".into()],
                timeout: Duration::from_secs(60),
            }],
            1,
        )
        .await?;
    println!("{}", serde_json::to_string_pretty(&results)?);
    for result in &results {
        let session = engine.resume(result.session_id.parse()?).await?;
        let trace = Trace::project(&session.events_after(0).await?)?;
        println!("{}", serde_json::to_string_pretty(&trace)?);
    }
    if results
        .iter()
        .any(|r| r.error.is_some() || r.scores.iter().any(|s| !s.pass))
    {
        return Err("An evaluation case failed".into());
    }
    Ok(())
}
