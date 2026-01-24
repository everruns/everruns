//! Infinity Context Evaluation Framework
//!
//! Compares different context management capabilities for long conversations:
//! - Baseline: Send all messages (fails on long conversations)
//! - Naive trim: Drop oldest messages (loses context)
//! - Infinity: Trim + history query tool (proposed solution)

pub mod capabilities;
mod metrics;
mod report;
mod runner;
mod scenarios;
pub mod types;

use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use std::path::PathBuf;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Parser)]
#[command(name = "eval")]
#[command(about = "Evaluate context management capabilities for long conversations")]
struct Cli {
    /// Run a specific scenario by name
    #[arg(short, long)]
    scenario: Option<String>,

    /// Run with a specific capability only (baseline, naive_trim, infinity_context)
    #[arg(long)]
    capability: Option<String>,

    /// Directory containing scenario definitions
    #[arg(long, default_value = "scenarios")]
    scenarios_dir: PathBuf,

    /// Output directory for reports
    #[arg(long, default_value = "results")]
    output_dir: PathBuf,

    /// Model to use for evaluation
    #[arg(long, default_value = "claude-sonnet-4-20250514")]
    model: String,

    /// Context window size in tokens
    #[arg(long, default_value = "200000")]
    context_window: usize,

    /// Context budget percentage (0.0-1.0)
    #[arg(long, default_value = "0.7")]
    budget_percent: f64,

    /// Generate synthetic scenarios instead of loading from files
    #[arg(long)]
    synthetic: bool,

    /// Dry run - show what would be executed without calling LLM
    #[arg(long)]
    dry_run: bool,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("info")
    };
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(filter)
        .init();

    println!(
        "\n{}",
        "╔═══════════════════════════════════════════════════════════╗"
            .bright_cyan()
            .bold()
    );
    println!(
        "{}",
        "║       Infinity Context Evaluation Framework               ║"
            .bright_cyan()
            .bold()
    );
    println!(
        "{}\n",
        "╚═══════════════════════════════════════════════════════════╝"
            .bright_cyan()
            .bold()
    );

    let config = runner::EvalConfig {
        model: cli.model.clone(),
        context_window: cli.context_window,
        budget_percent: cli.budget_percent,
        dry_run: cli.dry_run,
    };

    println!("{}", "Configuration:".bold());
    println!("  Model: {}", cli.model.bright_yellow());
    println!(
        "  Context window: {} tokens",
        cli.context_window.to_string().bright_yellow()
    );
    println!(
        "  Budget: {}%",
        (cli.budget_percent * 100.0).to_string().bright_yellow()
    );
    if cli.dry_run {
        println!("  {} Dry run mode", "⚠".bright_yellow());
    }
    println!();

    let scenarios = if cli.synthetic {
        println!("{} Generating synthetic scenarios...\n", "→".bright_blue());
        scenarios::generate_synthetic()
    } else {
        println!(
            "{} Loading scenarios from {:?}...\n",
            "→".bright_blue(),
            cli.scenarios_dir
        );
        scenarios::load_from_directory(&cli.scenarios_dir).await?
    };

    let scenarios: Vec<_> = if let Some(ref name) = cli.scenario {
        scenarios.into_iter().filter(|s| s.name == *name).collect()
    } else {
        scenarios
    };

    if scenarios.is_empty() {
        println!("{}", "No scenarios found!".bright_red());
        return Ok(());
    }

    println!(
        "{} Found {} scenario(s)\n",
        "✓".bright_green(),
        scenarios.len()
    );

    let capabilities = if let Some(ref name) = cli.capability {
        let all = runner::all_capabilities();
        let cap = all
            .into_iter()
            .find(|c| c.name() == name)
            .ok_or_else(|| anyhow::anyhow!("Unknown capability: {}", name))?;
        vec![cap]
    } else {
        runner::all_capabilities()
    };

    println!("{}", "Capabilities to evaluate:".bold());
    for cap in &capabilities {
        println!("  • {}", cap.name().bright_cyan());
    }
    println!();

    let results = runner::run_evaluation(&config, &scenarios, &capabilities).await?;

    let report = report::generate(&results, &config);
    println!("{}", report);

    let report_path = cli.output_dir.join(format!(
        "eval_{}_{}.md",
        chrono::Utc::now().format("%Y%m%d_%H%M%S"),
        cli.model.replace('/', "_")
    ));
    std::fs::create_dir_all(&cli.output_dir)?;
    std::fs::write(&report_path, &report)?;
    println!(
        "\n{} Report saved to: {:?}",
        "✓".bright_green(),
        report_path
    );

    Ok(())
}
