use weekend_concierge_host::run_weekend_concierge_demo;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let run = run_weekend_concierge_demo().await?;

    println!("== Runtime Tools ==");
    for tool in &run.tool_names {
        println!("- {tool}");
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
