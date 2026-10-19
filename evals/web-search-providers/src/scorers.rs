//! Scorers over the judge's verdict.
//!
//! - **`answerable`**: could an agent meet the need from this list? The
//!   headline number: it is what decides whether the agent answers well.
//! - **`relevance`**: nDCG of the graded list (value 0 to 1), passing at 0.5.
//!   Separates a list with one good hit buried at 5 from one that leads with it.
//! - **`current`**: on time-sensitive queries only, whether the list shows the
//!   current state. N/A elsewhere.
//! - **`returned`**: the search succeeded with at least one result. A failure
//!   counts against every other scorer too, as it would for an agent.

use mira::scorer::scorer;
use mira::{Sample, Score, Scorer, Transcript};

use crate::subject::{ERROR_KEY, SKIPPED_KEY, fresh};

fn skipped(name: &str, t: &Transcript) -> Option<Score> {
    t.metadata
        .get(SKIPPED_KEY)
        .and_then(|v| v.as_str())
        .map(|reason| Score::na(name, reason.to_string()))
}

pub fn answerable() -> Box<dyn Scorer> {
    scorer("answerable", |_: &Sample, t: &Transcript| {
        if let Some(na) = skipped("answerable", t) {
            return na;
        }
        match t.metadata.get("answerable").and_then(|v| v.as_bool()) {
            Some(true) => Score::pass("answerable", "the list meets the need"),
            Some(false) => Score::fail("answerable", "the list does not meet the need"),
            None => Score::na("answerable", "no verdict"),
        }
    })
}

pub fn relevance() -> Box<dyn Scorer> {
    scorer("relevance", |_: &Sample, t: &Transcript| {
        if let Some(na) = skipped("relevance", t) {
            return na;
        }
        let Some(ndcg) = t.metadata.get("ndcg").and_then(|v| v.as_f64()) else {
            return Score::na("relevance", "no verdict");
        };
        let mut score = if ndcg >= 0.5 {
            Score::pass("relevance", format!("nDCG {ndcg:.2}"))
        } else {
            Score::fail("relevance", format!("nDCG {ndcg:.2}"))
        };
        score.value = ndcg;
        score
    })
}

pub fn current() -> Box<dyn Scorer> {
    scorer("current", |sample: &Sample, t: &Transcript| {
        if !fresh(sample) {
            return Score::na("current", "not time-sensitive");
        }
        if let Some(na) = skipped("current", t) {
            return na;
        }
        match t.metadata.get("current").and_then(|v| v.as_bool()) {
            Some(true) => Score::pass("current", "shows the current state"),
            Some(false) => Score::fail("current", "outdated or undated"),
            None => Score::na("current", "no verdict"),
        }
    })
}

pub fn returned() -> Box<dyn Scorer> {
    scorer("returned", |_: &Sample, t: &Transcript| {
        if let Some(reason) = t
            .metadata
            .get(SKIPPED_KEY)
            .and_then(|v| v.as_str())
            .filter(|r| !r.starts_with("judge"))
        {
            return Score::na("returned", reason.to_string());
        }
        if let Some(error) = t.metadata.get(ERROR_KEY).and_then(|v| v.as_str()) {
            return Score::fail("returned", error.to_string());
        }
        let count = t
            .metadata
            .get("hits")
            .and_then(|v| v.as_array())
            .map_or(0, Vec::len);
        if count == 0 {
            Score::fail("returned", "no results")
        } else {
            Score::pass("returned", format!("{count} results"))
        }
    })
}
