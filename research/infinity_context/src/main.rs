//! Infinity Context Evaluation Framework
//!
//! Compares different context management capabilities for long conversations:
//! - Baseline: Send all messages (fails on long conversations)
//! - Naive trim: Drop oldest messages (loses context)
//! - Infinity: Trim + history query tool (proposed solution)
//!
//! Usage:
//!   eval generate -o datasets/v1.jsonl     # Generate dataset
//!   eval run -d datasets/v1.jsonl          # Run evaluation
//!   eval results -i results/run.jsonl      # View results

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

        /// Output results file (JSONL format)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Run a specific scenario by name
        #[arg(short, long)]
        scenario: Option<String>,

        /// Run with a specific capability only
        #[arg(long)]
        capability: Option<String>,

        /// Model to use for evaluation
        #[arg(long, default_value = "claude-sonnet-4-20250514")]
        model: String,

        /// Context window size in tokens
        #[arg(long, default_value = "200000")]
        context_window: usize,

        /// Context budget percentage (0.0-1.0)
        #[arg(long, default_value = "0.7")]
        budget_percent: f64,

        /// Dry run - show what would be executed without calling LLM
        #[arg(long)]
        dry_run: bool,
    },

    /// View evaluation results
    Results {
        /// Input results file (JSONL format)
        #[arg(short, long)]
        input: PathBuf,

        /// Output format (text, json, markdown)
        #[arg(long, default_value = "text")]
        format: String,
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
            output,
            scenario,
            capability,
            model,
            context_window,
            budget_percent,
            dry_run,
        } => {
            cmd_run(
                dataset,
                output,
                scenario,
                capability,
                model,
                context_window,
                budget_percent,
                dry_run,
            )
            .await
        }
        Commands::Results { input, format } => cmd_results(input, format).await,
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

/// Run evaluation against dataset
#[allow(clippy::too_many_arguments)]
async fn cmd_run(
    dataset_path: PathBuf,
    output: Option<PathBuf>,
    scenario_filter: Option<String>,
    capability_filter: Option<String>,
    model: String,
    context_window: usize,
    budget_percent: f64,
    dry_run: bool,
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

    // Save results as JSONL
    let output_path = output.unwrap_or_else(|| {
        PathBuf::from(format!(
            "results/eval_{}_{}.jsonl",
            Utc::now().format("%Y%m%d_%H%M%S"),
            model.replace('/', "_")
        ))
    });

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

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

    Ok(())
}

/// View evaluation results
async fn cmd_results(input: PathBuf, format: String) -> Result<()> {
    println!(
        "{} Loading results from: {}\n",
        "→".bright_blue(),
        input.display().to_string().bright_yellow()
    );

    let (metadata, results) = dataset::read_results(&input)?;

    match format.as_str() {
        "json" => {
            let output = serde_json::json!({
                "metadata": metadata,
                "results": results
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        "markdown" | "md" => {
            print_results_markdown(&metadata, &results);
        }
        _ => {
            print_results_text(&metadata, &results);
        }
    }

    Ok(())
}

fn print_results_text(metadata: &RunMetadata, results: &[EvalResultRecord]) {
    println!("{}", "Run Metadata:".bold());
    println!("  Timestamp: {}", metadata.timestamp);
    println!("  Model: {}", metadata.model.bright_yellow());
    println!("  Dataset: {}", metadata.dataset);
    println!(
        "  Context: {} tokens @ {}%",
        metadata.context_window,
        metadata.budget_percent * 100.0
    );
    println!();

    // Group by capability
    let mut by_capability: std::collections::HashMap<String, Vec<&EvalResultRecord>> =
        std::collections::HashMap::new();
    for r in results {
        by_capability
            .entry(r.capability.clone())
            .or_default()
            .push(r);
    }

    for (capability, cap_results) in &by_capability {
        let avg_score: f64 =
            cap_results.iter().map(|r| r.score).sum::<f64>() / cap_results.len() as f64;
        let passed_count = cap_results.iter().filter(|r| r.passed()).count();
        let total = cap_results.len();

        println!(
            "{}: {:.1}% avg ({}/{} passed)",
            capability.bright_cyan(),
            avg_score * 100.0,
            passed_count,
            total
        );

        for r in cap_results {
            let status = if r.score >= 0.8 {
                "✓".bright_green()
            } else if r.score >= 0.5 {
                "◐".bright_yellow()
            } else {
                "✗".bright_red()
            };
            println!(
                "  {} {} (score: {:.2}, {}ms, {} queries)",
                status, r.id, r.score, r.metrics.latency_ms, r.metrics.history_queries
            );
            // Show individual scorer results
            for s in &r.scores {
                let score_status = if s.value >= 0.5 { "✓" } else { "✗" };
                println!(
                    "      {} {}: {:.2}",
                    score_status.dimmed(),
                    s.name.dimmed(),
                    s.value
                );
            }
            if let Some(ref err) = r.error {
                println!("    Error: {}", err.dimmed());
            }
        }
        println!();
    }
}

fn print_results_markdown(metadata: &RunMetadata, results: &[EvalResultRecord]) {
    println!("# Evaluation Results\n");
    println!("**Timestamp:** {}  ", metadata.timestamp);
    println!("**Model:** {}  ", metadata.model);
    println!("**Dataset:** {}  ", metadata.dataset);
    println!(
        "**Context:** {} tokens @ {:.0}%\n",
        metadata.context_window,
        metadata.budget_percent * 100.0
    );

    // Summary table
    println!("## Summary\n");
    println!("| Capability | Avg Score | Passed | Failed |");
    println!("|------------|-----------|--------|--------|");

    let mut by_capability: std::collections::HashMap<String, Vec<&EvalResultRecord>> =
        std::collections::HashMap::new();
    for r in results {
        by_capability
            .entry(r.capability.clone())
            .or_default()
            .push(r);
    }

    for (capability, cap_results) in &by_capability {
        let avg_score: f64 =
            cap_results.iter().map(|r| r.score).sum::<f64>() / cap_results.len().max(1) as f64;
        let passed_count = cap_results.iter().filter(|r| r.passed()).count();
        let fail_count = cap_results.len() - passed_count;
        println!(
            "| {} | {:.1}% | {} | {} |",
            capability,
            avg_score * 100.0,
            passed_count,
            fail_count
        );
    }

    println!("\n## Detailed Results\n");
    for r in results {
        let status = if r.score >= 0.8 {
            "✅"
        } else if r.score >= 0.5 {
            "⚠️"
        } else {
            "❌"
        };
        println!(
            "- {} **{}** / {} - score: {:.2}, {}ms, {} queries",
            status, r.id, r.capability, r.score, r.metrics.latency_ms, r.metrics.history_queries
        );
        // Show individual scores
        for s in &r.scores {
            println!("  - {}: {:.2}", s.name, s.value);
        }
    }
}
