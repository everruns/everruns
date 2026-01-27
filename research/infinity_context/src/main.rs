//! Infinity Context Evaluation Framework
//!
//! Compares different context management capabilities for long conversations:
//! - Baseline: Send all messages (fails on long conversations)
//! - Naive trim: Drop oldest messages (loses context)
//! - Infinity: Trim + history query tool (proposed solution)
//!
//! Usage:
//!   eval generate -o datasets/v1.jsonl     # Generate dataset
//!   eval run -d datasets/v1.jsonl --save   # Run and save results

pub mod capabilities;
mod dataset;
mod metrics;
mod report;
mod runner;
mod scenarios;
pub mod scorer;
pub mod types;

use anyhow::Result;
use chrono::Utc;
use clap::{Parser, Subcommand};
use colored::Colorize;
use std::path::PathBuf;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use dataset::{EvalResultRecord, RunMetadata};
use types::Scenario;

#[derive(Parser)]
#[command(name = "eval")]
#[command(about = "Evaluate context management capabilities for long conversations")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Verbose output
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate evaluation dataset
    Generate {
        /// Output file path (JSONL format)
        #[arg(short, long, default_value = "datasets/infinity_context.jsonl")]
        output: PathBuf,
    },

    /// Run evaluation against a dataset
    Run {
        /// Input dataset file (JSONL format)
        #[arg(short, long)]
        dataset: PathBuf,

        /// Run a specific scenario by name
        #[arg(short, long)]
        scenario: Option<String>,

        /// Run with a specific capability only
        #[arg(long)]
        capability: Option<String>,

        /// Model to use for evaluation
        #[arg(long, default_value = "gpt-5.2")]
        model: String,

        /// Context window size in tokens (default 50k to ensure scenarios exceed budget)
        #[arg(long, default_value = "50000")]
        context_window: usize,

        /// Context budget percentage (0.0-1.0) - with 50k window, 0.7 = 35k token budget
        #[arg(long, default_value = "0.7")]
        budget_percent: f64,

        /// Dry run - show what would be executed without calling LLM
        #[arg(long)]
        dry_run: bool,

        /// Delay in milliseconds between scenarios (helps avoid rate limits)
        #[arg(long, default_value = "0")]
        delay_ms: u64,

        /// Delay in milliseconds between LLM calls within a scenario (for tool loops)
        #[arg(long, default_value = "0")]
        inter_call_delay_ms: u64,

        /// Save results to results/ directory
        #[arg(long)]
        save: bool,

        /// Custom moniker for result file naming (e.g., "baseline-run", "after-fix")
        #[arg(long)]
        moniker: Option<String>,
    },
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

    print_header();

    match cli.command {
        Commands::Generate { output } => cmd_generate(output).await,
        Commands::Run {
            dataset,
            scenario,
            capability,
            model,
            context_window,
            budget_percent,
            dry_run,
            delay_ms,
            inter_call_delay_ms,
            save,
            moniker,
        } => {
            cmd_run(
                dataset,
                scenario,
                capability,
                model,
                context_window,
                budget_percent,
                dry_run,
                delay_ms,
                inter_call_delay_ms,
                save,
                moniker,
            )
            .await
        }
    }
}

fn print_header() {
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
}

/// Generate evaluation dataset
async fn cmd_generate(output: PathBuf) -> Result<()> {
    println!("{} Generating dataset...\n", "→".bright_blue());

    let records = scenarios::generate_synthetic();

    println!("{}", "Generated scenarios:".bold());
    for record in &records {
        let scenario_type = record.meta.scenario_type.as_deref().unwrap_or("unknown");
        println!(
            "  • {} ({})",
            record.id.bright_cyan(),
            scenario_type.dimmed()
        );
    }
    println!();

    // Create output directory if needed
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    dataset::write_dataset(&output, &records)?;

    println!(
        "{} Dataset saved to: {}",
        "✓".bright_green(),
        output.display().to_string().bright_yellow()
    );
    println!(
        "  {} records, {} bytes",
        records.len(),
        std::fs::metadata(&output)?.len()
    );

    Ok(())
}

/// Generate a short moniker from model name
/// "claude-sonnet-4-20250514" -> "sonnet-4"
/// "gpt-5.2-turbo" -> "gpt-5.2-turbo"
fn generate_moniker(model: &str) -> String {
    // Handle Claude models
    if model.contains("claude") {
        // claude-sonnet-4-20250514 -> sonnet-4
        // claude-opus-4-5-20251101 -> opus-4.5
        let parts: Vec<&str> = model.split('-').collect();
        if parts.len() >= 3 {
            let variant = parts[1]; // sonnet, opus, haiku
            let version = if parts.len() >= 4 && parts[3].chars().all(|c| c.is_numeric()) {
                // Has sub-version like opus-4-5
                format!("{}.{}", parts[2], parts[3])
            } else {
                parts[2].to_string()
            };
            return format!("{}-{}", variant, version);
        }
    }
    // For other models, use as-is but remove date suffixes
    model
        .split('-')
        .take_while(|p| !p.chars().all(|c| c.is_numeric()) || p.len() < 8)
        .collect::<Vec<_>>()
        .join("-")
}

/// Run evaluation against dataset
#[allow(clippy::too_many_arguments)]
async fn cmd_run(
    dataset_path: PathBuf,
    scenario_filter: Option<String>,
    capability_filter: Option<String>,
    model: String,
    context_window: usize,
    budget_percent: f64,
    dry_run: bool,
    delay_ms: u64,
    inter_call_delay_ms: u64,
    save: bool,
    moniker: Option<String>,
) -> Result<()> {
    println!(
        "{} Loading dataset from: {}\n",
        "→".bright_blue(),
        dataset_path.display().to_string().bright_yellow()
    );

    let records = dataset::read_dataset(&dataset_path)?;
    let mut scenarios: Vec<Scenario> = records.into_iter().map(Scenario::from).collect();

    // Apply scenario filter
    if let Some(ref name) = scenario_filter {
        scenarios.retain(|s| s.name == *name);
    }

    if scenarios.is_empty() {
        println!("{}", "No scenarios found!".bright_red());
        return Ok(());
    }

    println!(
        "{} Found {} scenario(s)",
        "✓".bright_green(),
        scenarios.len()
    );
    for s in &scenarios {
        println!("  - {}", s.name.dimmed());
    }
    println!();

    let config = runner::EvalConfig {
        model: model.clone(),
        context_window,
        budget_percent,
        dry_run,
        delay_ms,
        inter_call_delay_ms,
    };

    println!("{}", "Configuration:".bold());
    println!("  Model: {}", model.bright_yellow());
    println!(
        "  Context window: {} tokens",
        context_window.to_string().bright_yellow()
    );
    println!(
        "  Budget: {}%",
        (budget_percent * 100.0).to_string().bright_yellow()
    );
    if dry_run {
        println!("  {} Dry run mode", "⚠".bright_yellow());
    }
    if delay_ms > 0 {
        println!(
            "  Delay: {}ms between scenarios",
            delay_ms.to_string().bright_yellow()
        );
    }
    if inter_call_delay_ms > 0 {
        println!(
            "  Inter-call delay: {}ms between LLM calls",
            inter_call_delay_ms.to_string().bright_yellow()
        );
    }
    println!();

    let capabilities = if let Some(ref name) = capability_filter {
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

    // Print report
    let report = report::generate(&results, &config);
    println!("{}", report);

    // Save results if requested
    if save {
        // Build filename: YYYYMMDD_HHMMSS__moniker.jsonl
        // Moniker defaults to model name (e.g., "sonnet-4.5", "gpt-5.2")
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let moniker = moniker.unwrap_or_else(|| generate_moniker(&model));
        let filename = format!("{}__{}.jsonl", timestamp, moniker);
        let output_path = PathBuf::from("results").join(&filename);

        std::fs::create_dir_all("results")?;

        // Convert results to JSONL records
        let mut result_records = Vec::new();
        for strategy_result in &results.strategy_results {
            for scenario_result in &strategy_result.scenario_results {
                result_records.push(EvalResultRecord {
                    id: scenario_result.scenario_name.clone(),
                    capability: scenario_result.strategy_name.clone(),
                    score: scenario_result.score,
                    scores: scenario_result.scores.clone(),
                    response: scenario_result.response.clone(),
                    error: scenario_result.error.clone(),
                    metrics: scenario_result.metrics.clone().into(),
                });
            }
        }

        let metadata = RunMetadata {
            timestamp: results.timestamp,
            model,
            context_window,
            budget_percent,
            dataset: dataset_path.display().to_string(),
            scenario_count: scenarios.len(),
            capability_count: capabilities.len(),
            capabilities: capabilities.iter().map(|c| c.name().to_string()).collect(),
        };

        dataset::write_results(&output_path, &metadata, &result_records)?;

        println!(
            "\n{} Results saved to: {}",
            "✓".bright_green(),
            output_path.display().to_string().bright_yellow()
        );

        // Also save markdown report
        let md_path = output_path.with_extension("md");
        std::fs::write(&md_path, &report)?;
        println!(
            "{} Report saved to: {}",
            "✓".bright_green(),
            md_path.display().to_string().bright_yellow()
        );
    }

    Ok(())
}
