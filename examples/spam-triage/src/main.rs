#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Run from the repository checkout; see README.md for credentials and the corpus.

mod dataset;
mod pipeline;
mod report;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clap::Parser;
use everruns::{Classifier, Model, TypeSafeAI};

use pipeline::{ADJUDICATION_MODEL, DEFAULT_CONFIDENCE_FLOOR, SCREENING_MODEL, Triage};
use report::{ROW_HEADER, Row, Tally};

/// Written by `data/build-corpus.sh`; gitignored, because the messages belong
/// to the people who sent them.
const DEFAULT_CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/data/corpus.jsonl");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse();
    let mut emails = dataset::load(&options.corpus)?;
    if let Some(limit) = options.limit {
        emails.truncate(limit);
    }

    // TYPESAFE_API_KEY and OPENROUTER_API_KEY, each declared by its own client.
    let classifier = Classifier::new(SCREENING_MODEL, TypeSafeAI::from_env()?);
    let model = Model::new(
        ADJUDICATION_MODEL,
        everruns_openrouter::from_env("openrouter")?,
    );

    println!("SCREEN      {SCREENING_MODEL} (TypeSafe)");
    println!("ADJUDICATE  {ADJUDICATION_MODEL} (OpenRouter)");
    println!(
        "CORPUS      {} — {} emails, escalating below {:.2} confidence\n",
        relative(&options.corpus),
        emails.len(),
        options.floor,
    );

    print!("screening {} emails... ", emails.len());
    io::stdout().flush()?;
    let started = Instant::now();
    let screenings = pipeline::screen_all(&classifier, &emails).await?;
    let screening_time = started.elapsed();
    let undecided: Vec<usize> = screenings
        .iter()
        .enumerate()
        .filter(|(_, screening)| !screening.is_settled(options.floor))
        .map(|(index, _)| index)
        .collect();
    println!(
        "{} answered in {}, {} below the floor",
        screenings.len(),
        seconds(screening_time),
        undecided.len(),
    );

    print!("adjudicating {} uncertain emails... ", undecided.len());
    io::stdout().flush()?;
    let started = Instant::now();
    let escalated: Vec<&dataset::Email> = undecided.iter().map(|index| &emails[*index]).collect();
    let adjudications = pipeline::adjudicate_all(&model, &escalated).await?;
    let adjudication_time = started.elapsed();
    println!(
        "{} answered in {}\n",
        adjudications.len(),
        seconds(adjudication_time)
    );

    let mut triages: Vec<Triage> = emails
        .into_iter()
        .zip(screenings)
        .map(|(email, screening)| Triage {
            email,
            screening,
            adjudication: None,
        })
        .collect();
    for (index, adjudication) in undecided.into_iter().zip(adjudications) {
        triages[index].adjudication = Some(adjudication);
    }

    println!("{ROW_HEADER}");
    for triage in &triages {
        println!("{}", Row(triage));
    }

    summarize(&triages, screening_time, adjudication_time);
    Ok(())
}

/// Print what the run is judged on: the accuracy routing bought, and its price.
fn summarize(triages: &[Triage], screening_time: Duration, adjudication_time: Duration) {
    let tally = Tally::of(triages);
    let screening_tokens: u64 = triages
        .iter()
        .map(|triage| triage.screening.input_tokens)
        .sum();
    let adjudication_tokens: u64 = triages
        .iter()
        .filter_map(|triage| triage.adjudication.as_ref())
        .filter_map(|adjudication| adjudication.total_tokens)
        .map(u64::from)
        .sum();
    let adjudication_cost: f64 = triages
        .iter()
        .filter_map(|triage| triage.adjudication.as_ref())
        .filter_map(|adjudication| adjudication.cost_usd)
        .sum();

    println!("\nSUMMARY");
    println!(
        "  screen alone    {:>3}/{:<3} {:>6.1}%",
        tally.screen_only_correct,
        tally.total,
        tally.screen_only_accuracy(),
    );
    println!(
        "  with routing    {:>3}/{:<3} {:>6.1}%",
        tally.pipeline_correct,
        tally.total,
        tally.pipeline_accuracy(),
    );
    println!(
        "  routing         {} settled, {} escalated ({} rescued, {} broken, {} unreadable)",
        tally.settled(),
        tally.escalated,
        tally.rescued,
        tally.broken,
        tally.unreadable,
    );
    println!(
        "  screening       {:>6} {:>10} input tokens",
        seconds(screening_time),
        screening_tokens,
    );
    println!(
        "  adjudication    {:>6} {:>10} tokens{}",
        seconds(adjudication_time),
        adjudication_tokens,
        // OpenRouter reports a per-generation cost inline; other providers do not.
        if adjudication_cost > 0.0 {
            format!(", ${adjudication_cost:.4}")
        } else {
            String::new()
        },
    );
}

/// Print the corpus path as it was typed from the repository root, so a
/// recorded run does not carry someone's home directory.
fn relative(path: &Path) -> String {
    std::env::current_dir()
        .ok()
        .and_then(|directory| path.strip_prefix(directory).ok())
        .unwrap_or(path)
        .display()
        .to_string()
}

fn seconds(elapsed: Duration) -> String {
    format!("{:.2}s", elapsed.as_secs_f64())
}

/// Screen a labeled email corpus with a fast classifier, then route only the
/// cases it was unsure about to a larger model.
// The three knobs worth exposing: which corpus, how much of it, and where the
// escalation threshold sits.
#[derive(Debug, PartialEq, Parser)]
struct Options {
    /// Labeled JSONL corpus to triage, as `data/build-corpus.sh` writes it.
    #[arg(long, value_name = "PATH", default_value = DEFAULT_CORPUS)]
    corpus: PathBuf,

    /// Triage only the first N emails, for a shorter and cheaper run.
    #[arg(long, value_name = "N", value_parser = parse_limit)]
    limit: Option<usize>,

    /// Escalate an email when the screen is less sure than this.
    #[arg(
        long = "threshold",
        value_name = "0.5-1.0",
        default_value_t = DEFAULT_CONFIDENCE_FLOOR,
        value_parser = parse_threshold,
    )]
    floor: f64,
}

fn parse_limit(value: &str) -> Result<usize, String> {
    match value.parse::<usize>() {
        Ok(0) => Err("must be at least 1".to_owned()),
        Ok(limit) => Ok(limit),
        Err(error) => Err(error.to_string()),
    }
}

fn parse_threshold(value: &str) -> Result<f64, String> {
    let floor: f64 = value.parse().map_err(|_| "must be a number".to_owned())?;
    // Confidence is `max(p, 1 - p)`, so it cannot fall below 0.5: a lower floor
    // would leave nothing uncertain and never escalate.
    if !(0.5..=1.0).contains(&floor) {
        return Err("must be between 0.5 and 1.0".to_owned());
    }
    Ok(floor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, clap::Error> {
        Options::try_parse_from(std::iter::once("spam-triage").chain(args.iter().copied()))
    }

    #[test]
    fn the_command_line_grammar_is_well_formed() {
        // Catches the derive mistakes clap only reports at runtime, such as two
        // flags claiming the same name.
        use clap::CommandFactory;
        Options::command().debug_assert();
    }

    #[test]
    fn defaults_point_at_the_built_corpus_and_the_calibrated_floor() {
        let options = parse(&[]).unwrap();
        assert!(options.corpus.ends_with("data/corpus.jsonl"));
        assert_eq!(options.floor, DEFAULT_CONFIDENCE_FLOOR);
        assert_eq!(options.limit, None);
    }

    #[test]
    fn every_flag_is_read() {
        let options = parse(&[
            "--corpus",
            "/tmp/c.jsonl",
            "--limit",
            "20",
            "--threshold",
            "0.8",
        ])
        .unwrap();
        assert_eq!(options.corpus, PathBuf::from("/tmp/c.jsonl"));
        assert_eq!(options.limit, Some(20));
        assert_eq!(options.floor, 0.8);
    }

    #[test]
    fn a_threshold_that_could_never_escalate_is_rejected() {
        assert!(parse(&["--threshold", "0.4"]).is_err());
        assert!(parse(&["--threshold", "1.5"]).is_err());
        assert!(parse(&["--threshold", "0.5"]).is_ok());
        assert!(parse(&["--threshold", "1.0"]).is_ok());
    }

    #[test]
    fn a_malformed_invocation_names_what_is_wrong() {
        for (args, expected) in [
            (vec!["--limit"], "--limit"),
            (vec!["--limit", "0"], "at least 1"),
            (vec!["--limit", "many"], "invalid digit"),
            (vec!["--threshold", "high"], "must be a number"),
            (vec!["--jev"], "--jev"),
        ] {
            let error = parse(&args).unwrap_err().to_string();
            assert!(error.contains(expected), "{args:?} reported: {error}");
        }
    }

    #[test]
    fn a_path_inside_the_working_directory_prints_without_it() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            relative(&cwd.join("data/corpus.jsonl")),
            "data/corpus.jsonl"
        );
        assert_eq!(
            relative(Path::new("/elsewhere/c.jsonl")),
            "/elsewhere/c.jsonl"
        );
    }

    #[test]
    fn elapsed_time_reads_in_seconds() {
        assert_eq!(seconds(Duration::from_millis(1420)), "1.42s");
    }
}
