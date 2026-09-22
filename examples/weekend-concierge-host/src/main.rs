use weekend_concierge_host::terminal::TerminalResponder;
use weekend_concierge_host::{run_weekend_concierge, run_weekend_concierge_demo};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // The agent asks before it plans. Answer at the prompt; piping input or
    // passing --scripted uses the same answers the offline test uses, so this
    // stays runnable in CI and in a pipeline.
    let scripted = std::env::args().any(|arg| arg == "--scripted");
    let run = if scripted {
        run_weekend_concierge_demo().await?
    } else {
        run_weekend_concierge(TerminalResponder::new()).await?
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
