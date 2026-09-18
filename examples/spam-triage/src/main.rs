#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Run from the repository checkout; see README.md for credentials and the corpus.

mod dataset;
mod pipeline;
mod report;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use everruns::{Classifier, Model, TypeSafeAI};

use pipeline::{ADJUDICATION_MODEL, DEFAULT_CONFIDENCE_FLOOR, SCREENING_MODEL, Triage};
use report::{ROW_HEADER, Row, Tally};

/// Written by `data/build-corpus.sh`; gitignored, because the messages belong
/// to the people who sent them.
const DEFAULT_CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/data/corpus.jsonl");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse(std::env::args().skip(1))?;
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

/// The three knobs worth exposing: which corpus, how much of it, and where the
/// escalation threshold sits.
#[derive(Debug, PartialEq)]
struct Options {
    corpus: PathBuf,
    limit: Option<usize>,
    floor: f64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            corpus: PathBuf::from(DEFAULT_CORPUS),
            limit: None,
            floor: DEFAULT_CONFIDENCE_FLOOR,
        }
    }
}

impl Options {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, io::Error> {
        let mut options = Options::default();
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            let mut value = || {
                args.next().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, format!("{flag} needs a value"))
                })
            };
            match flag.as_str() {
                "--corpus" => options.corpus = PathBuf::from(value()?),
                "--limit" => options.limit = Some(parse_with(&flag, &value()?)?),
                "--threshold" => options.floor = parse_with(&flag, &value()?)?,
                other => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "unknown argument '{other}'\n\
                             Usage: spam-triage [--corpus PATH] [--limit N] [--threshold 0.0-1.0]"
                        ),
                    ));
                }
            }
        }
        if !(0.5..=1.0).contains(&options.floor) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "--threshold must be between 0.5 and 1.0; below 0.5 nothing is uncertain",
            ));
        }
        if options.limit == Some(0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "--limit must be at least 1",
            ));
        }
        Ok(options)
    }
}

fn parse_with<T: std::str::FromStr>(flag: &str, value: &str) -> Result<T, io::Error> {
    value.parse().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{flag}: '{value}' is not a valid value"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, io::Error> {
        Options::parse(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn defaults_point_at_the_built_corpus_and_the_calibrated_floor() {
        let options = parse(&[]).unwrap();
        assert_eq!(options, Options::default());
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
        for args in [
            vec!["--limit"],
            vec!["--limit", "0"],
            vec!["--limit", "many"],
            vec!["--jev"],
        ] {
            assert_eq!(
                parse(&args).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{args:?}"
            );
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
