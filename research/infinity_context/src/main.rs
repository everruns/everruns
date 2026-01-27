//! Infinity Context Evaluation Framework
//!
//! Compares different context management approaches for long conversations:
//! - Baseline: Send all messages (fails on long conversations)
//! - Naive trim: Drop oldest messages (loses context)
//! - Infinity: Trim + history query tool (proposed solution)
//!
//! Usage:
//!   eval run --save                        # Run evals (dataset is in git)
//!   eval run --model claude-sonnet-4-20250514 --save
//!   eval generate                          # Regenerate dataset (rarely needed)

pub mod capabilities;
mod dataset;
mod report;
mod runner;
mod scenarios;
pub mod scorer;

use anyhow::Result;
use chrono::Utc;
use clap::{Parser, Subcommand};
use colored::Colorize;
use std::path::PathBuf;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use dataset::{DatasetRecord, EvalResultRecord, RunMetadata};
use runner::{Approach, EvaluationResults, aggregate_approach_results};

#[derive(Parser)]
#[command(name = "eval")]
#[command(about = "Evaluate context management approaches for long conversations")]
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
        #[arg(short, long, default_value = "datasets/infinity_context.jsonl")]
        dataset: PathBuf,

        /// Run a specific record by id
        #[arg(short, long)]
        record: Option<String>,

        /// Approach to evaluate (baseline, naive_trim, infinity_context)
        #[arg(long)]
        approach: Option<String>,

        /// Model to use for evaluation
        #[arg(long, default_value = "gpt-5.2")]
        model: String,

        /// Dry run - show what would be executed without calling LLM
        #[arg(long)]
        dry_run: bool,

        /// Delay in milliseconds between records (helps avoid rate limits)
        #[arg(long, default_value = "0")]
        delay_ms: u64,

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
    // Load .env from project root (two levels up from research/infinity_context)
    let _ = dotenvy::from_filename("../../.env");

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
            record,
            approach,
            model,
            dry_run,
            delay_ms,
            save,
            moniker,
        } => cmd_run(dataset, record, approach, model, dry_run, delay_ms, save, moniker).await,
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

    println!("{}", "Generated records:".bold());
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
    record_filter: Option<String>,
    approach_filter: Option<String>,
    model: String,
    dry_run: bool,
    delay_ms: u64,
    save: bool,
    moniker: Option<String>,
) -> Result<()> {
    println!(
        "{} Loading dataset from: {}\n",
        "→".bright_blue(),
        dataset_path.display().to_string().bright_yellow()
    );

    let mut records: Vec<DatasetRecord> = dataset::read_dataset(&dataset_path)?;

    // Apply record filter
    if let Some(ref id) = record_filter {
        records.retain(|r| r.id == *id);
    }

    if records.is_empty() {
        println!("{}", "No records found!".bright_red());
        return Ok(());
    }

    println!(
        "{} Found {} record(s)",
        "✓".bright_green(),
        records.len()
    );
    for r in &records {
        println!("  - {}", r.id.dimmed());
    }
    println!();

    println!("{}", "Configuration:".bold());
    println!("  Model: {}", model.bright_yellow());
    if dry_run {
        println!("  {} Dry run mode", "⚠".bright_yellow());
    }
    if delay_ms > 0 {
        println!(
            "  Delay: {}ms between records",
            delay_ms.to_string().bright_yellow()
        );
    }
    println!();

    // Get approaches to evaluate
    let approaches: Vec<Approach> = if let Some(ref name) = approach_filter {
        vec![name.parse()?]
    } else {
        Approach::all()
    };

    println!("{}", "Approaches to evaluate:".bold());
    for approach in &approaches {
        println!("  • {}", approach.name().bright_cyan());
    }
    println!();

    // Run evaluation for each approach
    let mut approach_results = Vec::new();

    for (idx, approach) in approaches.iter().enumerate() {
        // Delay between approaches
        if idx > 0 && delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }

        println!(
            "{} Approach: {}",
            "━".repeat(60).dimmed(),
            approach.name().bright_cyan().bold()
        );
        println!("  {}\n", approach.description().dimmed());

        let config = runner::EvalConfig {
            model: model.clone(),
            approach: *approach,
            dry_run,
        };

        // Phase 1: Run all records
        let mut run_results = Vec::new();
        for (record_idx, record) in records.iter().enumerate() {
            // Delay between records
            if record_idx > 0 && delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }

            let run_result = runner::run_scenario(&config, record).await;
            print_run_status(&run_result);
            run_results.push((record, run_result));
        }

        // Phase 2: Score all results
        println!("\n  {} Scoring results...", "→".bright_blue());
        let mut record_results = Vec::new();
        for (record, run_result) in &run_results {
            let scored = runner::score_result(record, run_result).await;
            record_results.push(scored);
        }

        // Aggregate and print summary
        let aggregated = aggregate_approach_results(approach.name(), record_results);
        println!(
            "\n  {} Avg Score: {:.1}% ({}/{})\n",
            "→".bright_blue(),
            aggregated.accuracy(),
            aggregated.passed,
            aggregated.total_records
        );

        approach_results.push(aggregated);
    }

    let results = EvaluationResults {
        timestamp: Utc::now(),
        model: model.clone(),
        config: std::collections::HashMap::new(),
        approach_results,
    };

    // Print report
    let report = report::generate(&results);
    println!("{}", report);

    // Save results if requested
    if save {
        save_results(&results, &dataset_path, &model, moniker)?;
    }

    Ok(())
}

fn print_run_status(run: &runner::RunResult) {
    let status = if run.error.is_some() {
        if run.metrics.context_exceeded {
            "⊘".bright_yellow()
        } else {
            "✗".bright_red()
        }
    } else {
        "●".bright_blue() // Running indicator (not scored yet)
    };

    println!(
        "  {} {} (tokens: {}, latency: {}ms, queries: {})",
        status,
        run.record_id,
        run.metrics.total_tokens,
        run.metrics.latency_ms,
        run.metrics.history_queries
    );

    if let Some(err) = &run.error {
        if !run.metrics.context_exceeded {
            println!("    {} {}", "Error:".red(), err);
        }
    }
}

fn save_results(
    results: &EvaluationResults,
    dataset_path: &PathBuf,
    model: &str,
    moniker: Option<String>,
) -> Result<()> {
    std::fs::create_dir_all("results")?;

    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let model_moniker = moniker.unwrap_or_else(|| generate_moniker(model));

    // Save separate file per approach
    for approach_result in &results.approach_results {
        let filename = format!(
            "{}__{}__{}.jsonl",
            timestamp, model_moniker, approach_result.approach_name
        );
        let output_path = PathBuf::from("results").join(&filename);

        // Convert to JSONL records
        let result_records: Vec<EvalResultRecord> = approach_result
            .record_results
            .iter()
            .map(|r| EvalResultRecord {
                id: r.record_id.clone(),
                capability: r.approach_name.clone(),
                score: r.score,
                scores: r.scores.clone(),
                response: r.response.clone(),
                error: r.error.clone(),
                metrics: r.metrics.clone().into(),
            })
            .collect();

        let metadata = RunMetadata {
            timestamp: results.timestamp,
            model: model.to_string(),
            dataset: dataset_path.display().to_string(),
            scenario_count: approach_result.total_records,
            capability_count: 1,
            capabilities: vec![approach_result.approach_name.clone()],
        };

        dataset::write_results(&output_path, &metadata, &result_records)?;

        println!(
            "{} Results saved to: {}",
            "✓".bright_green(),
            output_path.display().to_string().bright_yellow()
        );
    }

    // Save combined markdown report
    let md_filename = format!("{}__{}.md", timestamp, model_moniker);
    let md_path = PathBuf::from("results").join(&md_filename);
    let report = report::generate(results);
    std::fs::write(&md_path, &report)?;
    println!(
        "{} Report saved to: {}",
        "✓".bright_green(),
        md_path.display().to_string().bright_yellow()
    );

    Ok(())
}
