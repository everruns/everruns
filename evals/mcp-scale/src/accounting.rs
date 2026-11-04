//! Repair derived usage from retained transcripts without another model run.

use std::io::{self, BufRead};

use mira::Transcript;
use serde_json::{Value, json};

use crate::event_usage;

pub fn reaccount_report(input: &str, output: &str) -> io::Result<()> {
    let reader: Box<dyn BufRead> = if input == "-" {
        Box::new(io::BufReader::new(io::stdin()))
    } else {
        Box::new(io::BufReader::new(std::fs::File::open(input)?))
    };
    let mut cases = Vec::new();
    for line in reader.lines() {
        let row: Value = serde_json::from_str(&line?).map_err(io::Error::other)?;
        let events = row["events"]
            .as_array()
            .ok_or_else(|| io::Error::other("report row has no events"))?;
        let mut transcript = Transcript::default();
        event_usage::summarize_events(events).apply_to(&mut transcript);
        cases.push(json!({
            "case": row["case"],
            "original_usage": row["usage"],
            "usage": transcript.usage,
            "metrics": transcript.metrics,
            "metadata": transcript.metadata,
        }));
    }
    std::fs::write(output, serde_json::to_vec_pretty(&cases)?)
}
