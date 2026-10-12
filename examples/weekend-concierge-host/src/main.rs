use weekend_concierge_host::terminal::TerminalResponder;
use weekend_concierge_host::{ScriptedResponder, default_model, run_weekend_concierge};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // The agent runs on a real model (OPENAI_API_KEY) and asks before it
    // plans. Answer at the prompt; piping input or passing --scripted answers
    // without a person, so this stays runnable in a pipeline.
    let model = default_model()?;
    let scripted = std::env::args().any(|arg| arg == "--scripted");
    let run = if scripted {
        run_weekend_concierge(model, ScriptedResponder).await?
    } else {
        run_weekend_concierge(model, TerminalResponder::new()).await?
    };

    println!("== Runtime Tools ==");
    for tool in &run.tool_names {
        println!("- {tool}");
    }

    println!("\n== Your Answers ==");
    for answer in &run.answers {
        println!("- {answer}");
    }

    println!("\n== Seeded Brief ==");
    println!("{}", run.seeded_brief);

    println!("\n== Final Response ==");
    println!("{}", run.final_response);
    println!(
        "\nturn success: {} | iterations: {} | tool calls: {}",
        run.success, run.iterations, run.tool_calls_count
    );

    println!("\n== Transcript ==");
    for line in &run.transcript {
        println!("{line}");
    }

    println!("\n== Event Types ==");
    for event in &run.event_types {
        println!("- {event}");
    }

    Ok(())
}
