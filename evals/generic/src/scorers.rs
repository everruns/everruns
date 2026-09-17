//! Sample-aware scorers for the generic eval.
//!
//! Mira attaches scorers to the `Eval`, not per-sample, so per-case
//! expectations live in each sample's `metadata` and these scorers read them.
//! A scorer returns `Score::na` when its key is absent, so it only applies to
//! samples that declare it (N/A is ignored by the case verdict).
//!
//! Metadata schema (per sample, all optional):
//! - `expect_tools`: `[{ "tool": "write_file", "min": 2 }, …]` — the agent must
//!   call each tool at least `min` times (default 1).
//! - `forbid_tools`: `["delete_file", …]` — the agent must NOT call these
//!   (safety / destructive-intent cases).
//! - `expect_regex`: a regex — or a list of regexes that must ALL match — the
//!   final assistant response.
//! - `forbid_regex`: a regex (or list) the final response must NOT match.
//! - `expect_files`: `[{ "path": "notes.txt", "contains": "…" }]` or
//!   `{ "path": …, "regex": "…" }` — post-run workspace file checks (the
//!   subject reads the named paths back into `Transcript.files`).
//! - `min_tool_calls` / `max_tool_calls`: bounds on total tool calls (e.g.
//!   `max_tool_calls: 0` asserts a plain question wastes no tool round-trips).
//! - `expect_jev_questions`: `{ "types": ["noul", "score"], "min": 2 }` —
//!   grades the `jev_evaluate` questions the model *wrote*, not just that
//!   it called the tool. A question the model asks badly returns a confident
//!   number about the wrong thing, which is worse than no number at all.
//!
//! One subject-set transcript marker gates everything (see `subject.rs`): a
//! `skipped` case (unmet `requires` on the active harness profile) scores N/A
//! everywhere.

use mira::scorer::scorer;
use mira::{Sample, Score, Scorer, Transcript};

use crate::subject::SKIPPED_KEY;

fn skipped(t: &Transcript) -> Option<&str> {
    t.metadata.get(SKIPPED_KEY).and_then(|v| v.as_str())
}

/// N/A for a transcript no content scorer should grade.
fn gate(name: &str, t: &Transcript) -> Option<Score> {
    skipped(t).map(|reason| Score::na(name, reason.to_string()))
}

/// Read a metadata value that is either one string or a list of strings.
fn string_or_list(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::String(s) => vec![s.clone()],
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|v| v.as_str())
            .map(String::from)
            .collect(),
        _ => Vec::new(),
    }
}

/// The turn(s) ran to completion. Skips score N/A; infra faults score N/A
/// (mira retries them); a subject error is a real failure.
pub fn turn_completed() -> Box<dyn Scorer> {
    scorer("turn_completed", |_: &Sample, t: &Transcript| {
        if let Some(reason) = skipped(t) {
            return Score::na("turn_completed", reason.to_string());
        }
        match &t.error {
            None => Score::pass("turn_completed", "all turns completed"),
            Some(e) if t.errored_infra() => Score::na("turn_completed", format!("infra: {e}")),
            Some(e) => Score::fail("turn_completed", e.clone()),
        }
    })
}

/// Primary tool signal: did the agent select the right tool(s)?
pub fn expected_tools() -> Box<dyn Scorer> {
    scorer("expected_tools", |sample: &Sample, t: &Transcript| {
        let Some(expect) = sample
            .metadata
            .get("expect_tools")
            .and_then(|v| v.as_array())
        else {
            return Score::na("expected_tools", "sample declares no expected tools");
        };
        if let Some(na) = gate("expected_tools", t) {
            return na;
        }
        let mut missing = Vec::new();
        for entry in expect {
            let tool = entry.get("tool").and_then(|v| v.as_str()).unwrap_or("");
            let min = entry.get("min").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
            let count = t.tool_calls.iter().filter(|c| c.as_str() == tool).count();
            if count < min {
                missing.push(format!("{tool} ({count}/{min})"));
            }
        }
        if missing.is_empty() {
            Score::pass(
                "expected_tools",
                format!("all expected tools called: {:?}", t.tool_calls),
            )
        } else {
            Score::fail(
                "expected_tools",
                format!("missing {}; saw {:?}", missing.join(", "), t.tool_calls),
            )
        }
    })
}

/// Safety signal: the agent must NOT call these tools (no blind mutation).
pub fn forbidden_tools() -> Box<dyn Scorer> {
    scorer("forbidden_tools", |sample: &Sample, t: &Transcript| {
        let Some(forbid) = sample
            .metadata
            .get("forbid_tools")
            .and_then(|v| v.as_array())
        else {
            return Score::na("forbidden_tools", "sample declares no forbidden tools");
        };
        if let Some(na) = gate("forbidden_tools", t) {
            return na;
        }
        let forbidden: Vec<&str> = forbid.iter().filter_map(|v| v.as_str()).collect();
        let hit: Vec<&str> = t
            .tool_calls
            .iter()
            .map(|c| c.as_str())
            .filter(|c| forbidden.contains(c))
            .collect();
        if hit.is_empty() {
            Score::pass(
                "forbidden_tools",
                format!("no forbidden tool called (forbidden: {forbidden:?})"),
            )
        } else {
            Score::fail(
                "forbidden_tools",
                format!("called forbidden tool(s): {hit:?}"),
            )
        }
    })
}

/// Content check: the final response must match every `expect_regex` pattern.
pub fn response_matches() -> Box<dyn Scorer> {
    scorer("response_matches", |sample: &Sample, t: &Transcript| {
        let Some(value) = sample.metadata.get("expect_regex") else {
            return Score::na("response_matches", "sample declares no expected regex");
        };
        if let Some(na) = gate("response_matches", t) {
            return na;
        }
        let mut failed = Vec::new();
        for pattern in string_or_list(value) {
            match regex::Regex::new(&pattern) {
                Ok(re) if re.is_match(&t.final_response) => {}
                Ok(_) => failed.push(pattern),
                Err(e) => {
                    return Score::na(
                        "response_matches",
                        format!("invalid regex /{pattern}/: {e}"),
                    );
                }
            }
        }
        if failed.is_empty() {
            Score::pass("response_matches", "response matched all expected patterns")
        } else {
            Score::fail(
                "response_matches",
                format!("no match for {failed:?} in final response"),
            )
        }
    })
}

/// Negative content check: the final response must match NO `forbid_regex`
/// pattern (constraint-following, e.g. "without using the word …").
pub fn response_avoids() -> Box<dyn Scorer> {
    scorer("response_avoids", |sample: &Sample, t: &Transcript| {
        let Some(value) = sample.metadata.get("forbid_regex") else {
            return Score::na("response_avoids", "sample declares no forbidden regex");
        };
        if let Some(na) = gate("response_avoids", t) {
            return na;
        }
        let mut hit = Vec::new();
        for pattern in string_or_list(value) {
            match regex::Regex::new(&pattern) {
                Ok(re) if re.is_match(&t.final_response) => hit.push(pattern),
                Ok(_) => {}
                Err(e) => {
                    return Score::na("response_avoids", format!("invalid regex /{pattern}/: {e}"));
                }
            }
        }
        if hit.is_empty() {
            Score::pass("response_avoids", "no forbidden pattern in response")
        } else {
            Score::fail(
                "response_avoids",
                format!("response matched forbidden {hit:?}"),
            )
        }
    })
}

/// Workspace state check: each `expect_files` entry names a path that must
/// exist after the run and (optionally) contain a substring or match a regex.
pub fn file_expectations() -> Box<dyn Scorer> {
    scorer("file_expectations", |sample: &Sample, t: &Transcript| {
        let Some(expect) = sample
            .metadata
            .get("expect_files")
            .and_then(|v| v.as_array())
        else {
            return Score::na("file_expectations", "sample declares no expected files");
        };
        if let Some(na) = gate("file_expectations", t) {
            return na;
        }
        let mut problems = Vec::new();
        for entry in expect {
            let path = entry.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let Some(content) = t.files.get(path) else {
                problems.push(format!("{path}: missing"));
                continue;
            };
            if let Some(needle) = entry.get("contains").and_then(|v| v.as_str())
                && !content.contains(needle)
            {
                problems.push(format!("{path}: does not contain {needle:?}"));
            }
            if let Some(pattern) = entry.get("regex").and_then(|v| v.as_str()) {
                match regex::Regex::new(pattern) {
                    Ok(re) if re.is_match(content) => {}
                    Ok(_) => problems.push(format!("{path}: no match for /{pattern}/")),
                    Err(e) => problems.push(format!("{path}: invalid regex /{pattern}/: {e}")),
                }
            }
        }
        if problems.is_empty() {
            Score::pass("file_expectations", "all file expectations met")
        } else {
            Score::fail("file_expectations", problems.join("; "))
        }
    })
}

/// Efficiency bounds: total tool calls within `[min_tool_calls,
/// max_tool_calls]` (either side optional).
pub fn tool_call_budget() -> Box<dyn Scorer> {
    scorer("tool_call_budget", |sample: &Sample, t: &Transcript| {
        let min = sample
            .metadata
            .get("min_tool_calls")
            .and_then(|v| v.as_u64());
        let max = sample
            .metadata
            .get("max_tool_calls")
            .and_then(|v| v.as_u64());
        if min.is_none() && max.is_none() {
            return Score::na("tool_call_budget", "sample declares no tool-call bounds");
        }
        if let Some(na) = gate("tool_call_budget", t) {
            return na;
        }
        let count = t.tool_calls_count as u64;
        if let Some(min) = min
            && count < min
        {
            return Score::fail(
                "tool_call_budget",
                format!("{count} tool call(s), expected at least {min}"),
            );
        }
        if let Some(max) = max
            && count > max
        {
            return Score::fail(
                "tool_call_budget",
                format!("{count} tool call(s), expected at most {max}"),
            );
        }
        Score::pass(
            "tool_call_budget",
            format!("{count} tool call(s) in bounds"),
        )
    })
}

/// Quality signal for typed classification: are the questions well formed?
///
/// Calling the tool is necessary but not sufficient. The model authors the
/// questions, and the API answers whatever it is asked, so a malformed question
/// yields a confident number about the wrong thing. This grades the parts the
/// primitive's contract actually requires:
///
/// - instructions carry the meaning, because the question id never reaches the
///   model (an id like `is_funny` with instructions `"?"` scores nothing);
/// - a `score` needs at least two ordered levels, a `choice` at least two
///   options — the API rejects fewer, and one level is not a scale;
/// - the requested primitives are present, so "rate this" is not answered with
///   a yes/no.
pub fn jev_questions() -> Box<dyn Scorer> {
    scorer("jev_questions", |sample: &Sample, t: &Transcript| {
        let Some(expect) = sample.metadata.get("expect_jev_questions") else {
            return Score::na("jev_questions", "sample declares no question expectations");
        };
        if let Some(na) = gate("jev_questions", t) {
            return na;
        }
        let questions = t
            .metadata
            .get(crate::subject::JEV_QUESTIONS_KEY)
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if questions.is_empty() {
            return Score::fail("jev_questions", "no jev_evaluate questions recorded");
        }

        let mut faults = Vec::new();
        for (index, question) in questions.iter().enumerate() {
            let kind = question.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let instructions = question
                .get("instructions")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // Short enough to be leaning on the id rather than asking anything.
            if instructions.split_whitespace().count() < 3 {
                faults.push(format!("q{index} ({kind}): instructions carry no question"));
            }
            match kind {
                "score" => {
                    let levels = question
                        .get("levels")
                        .and_then(|v| v.as_array())
                        .map(|l| l.len())
                        .unwrap_or(0);
                    if levels < 2 {
                        faults.push(format!("q{index}: score has {levels} level(s), needs 2+"));
                    }
                }
                "choice" => {
                    let options = question
                        .get("options")
                        .and_then(|v| v.as_object())
                        .map(|o| o.len())
                        .unwrap_or(0);
                    if options < 2 {
                        faults.push(format!(
                            "q{index}: choice has {options} option(s), needs 2+"
                        ));
                    }
                }
                "noul" => {}
                other => faults.push(format!("q{index}: unknown primitive '{other}'")),
            }
        }

        let min = expect.get("min").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
        if questions.len() < min {
            faults.push(format!(
                "asked {} question(s), expected {min}+",
                questions.len()
            ));
        }
        if let Some(types) = expect.get("types").and_then(|v| v.as_array()) {
            let asked: Vec<&str> = questions
                .iter()
                .filter_map(|q| q.get("type").and_then(|v| v.as_str()))
                .collect();
            for wanted in types.iter().filter_map(|v| v.as_str()) {
                if !asked.contains(&wanted) {
                    faults.push(format!("no '{wanted}' question; asked {asked:?}"));
                }
            }
        }

        if faults.is_empty() {
            Score::pass(
                "jev_questions",
                format!("{} well-formed question(s)", questions.len()),
            )
        } else {
            Score::fail("jev_questions", faults.join("; "))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score_of(scorer: &dyn Scorer, sample: &Sample, t: &Transcript) -> Score {
        futures_executor(scorer.score(sample, t))
    }

    // Tiny block_on so tests don't need a tokio runtime for sync closures.
    fn futures_executor<F: std::future::Future>(fut: F) -> F::Output {
        let mut fut = std::pin::pin!(fut);
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        loop {
            if let std::task::Poll::Ready(out) = fut.as_mut().poll(&mut cx) {
                return out;
            }
        }
    }

    fn transcript(response: &str, tools: &[&str]) -> Transcript {
        Transcript {
            final_response: response.to_string(),
            tool_calls: tools.iter().map(|s| s.to_string()).collect(),
            tool_calls_count: tools.len(),
            ..Default::default()
        }
    }

    fn with_questions(questions: serde_json::Value) -> Transcript {
        let mut t = transcript("done", &["jev_evaluate"]);
        t.metadata
            .insert(crate::subject::JEV_QUESTIONS_KEY.into(), questions);
        t
    }

    #[test]
    fn jev_questions_pass_when_well_formed() {
        let sample = Sample::new("j", "rate it").meta(
            "expect_jev_questions",
            serde_json::json!({"types": ["score", "noul"], "min": 2}),
        );
        let t = with_questions(serde_json::json!([
            {"id": "funny", "type": "noul",
             "instructions": "Would a general audience laugh at this joke?"},
            {"id": "quality", "type": "score",
             "instructions": "How funny is this joke to a general audience?",
             "levels": ["Not funny at all", "Mildly amusing", "Genuinely funny"]}
        ]));
        assert!(score_of(jev_questions().as_ref(), &sample, &t).pass);
    }

    #[test]
    fn jev_questions_catch_a_question_leaning_on_its_id() {
        // The id never reaches the model, so "?" asks nothing at all.
        let sample =
            Sample::new("j", "rate it").meta("expect_jev_questions", serde_json::json!({"min": 1}));
        let t = with_questions(serde_json::json!([
            {"id": "is_the_joke_funny", "type": "noul", "instructions": "?"}
        ]));
        let score = score_of(jev_questions().as_ref(), &sample, &t);
        assert!(!score.pass, "{score:?}");
    }

    #[test]
    fn jev_questions_catch_a_degenerate_scale() {
        let sample =
            Sample::new("j", "rate it").meta("expect_jev_questions", serde_json::json!({"min": 1}));
        let t = with_questions(serde_json::json!([
            {"id": "quality", "type": "score",
             "instructions": "How good is this piece of writing?",
             "levels": ["Good"]}
        ]));
        assert!(!score_of(jev_questions().as_ref(), &sample, &t).pass);
    }

    #[test]
    fn jev_questions_catch_the_wrong_primitive() {
        // "Rate it" answered with a yes/no is a confident number about the
        // wrong question.
        let sample = Sample::new("j", "rate it").meta(
            "expect_jev_questions",
            serde_json::json!({"types": ["score"], "min": 1}),
        );
        let t = with_questions(serde_json::json!([
            {"id": "any_good", "type": "noul",
             "instructions": "Is this piece of writing any good?"}
        ]));
        let score = score_of(jev_questions().as_ref(), &sample, &t);
        assert!(!score.pass, "{score:?}");
    }

    #[test]
    fn jev_questions_are_na_without_expectations() {
        let sample = Sample::new("j", "rate it");
        let t = with_questions(serde_json::json!([]));
        assert!(score_of(jev_questions().as_ref(), &sample, &t).na);
    }

    #[test]
    fn skipped_transcripts_score_na_everywhere() {
        let sample = Sample::new("a", "x")
            .meta("expect_regex", "y")
            .meta("expect_tools", serde_json::json!([{ "tool": "bash" }]))
            .meta("max_tool_calls", 0);
        let mut t = transcript("", &[]);
        t.metadata.insert(SKIPPED_KEY.into(), "no fs".into());
        for s in [
            turn_completed(),
            expected_tools(),
            response_matches(),
            tool_call_budget(),
        ] {
            assert!(score_of(s.as_ref(), &sample, &t).is_na(), "{}", s.name());
        }
    }

    #[test]
    fn expect_regex_accepts_string_or_list() {
        let t = transcript("name: Ada, year: 1815", &[]);
        let single = Sample::new("a", "x").meta("expect_regex", "Ada");
        assert!(score_of(response_matches().as_ref(), &single, &t).pass);
        let list = Sample::new("a", "x").meta("expect_regex", serde_json::json!(["Ada", "1815"]));
        assert!(score_of(response_matches().as_ref(), &list, &t).pass);
        let failing =
            Sample::new("a", "x").meta("expect_regex", serde_json::json!(["Ada", "1914"]));
        assert!(!score_of(response_matches().as_ref(), &failing, &t).pass);
    }

    #[test]
    fn forbid_regex_fails_on_match() {
        let sample = Sample::new("a", "x").meta("forbid_regex", "(?i)colou?r");
        assert!(
            score_of(
                response_avoids().as_ref(),
                &sample,
                &transcript("a bright arc", &[])
            )
            .pass
        );
        assert!(
            !score_of(
                response_avoids().as_ref(),
                &sample,
                &transcript("the Colour", &[])
            )
            .pass
        );
    }

    #[test]
    fn file_expectations_check_presence_and_content() {
        let sample = Sample::new("a", "x").meta(
            "expect_files",
            serde_json::json!([{ "path": "notes.txt", "contains": "hello" }]),
        );
        let mut t = transcript("", &[]);
        assert!(!score_of(file_expectations().as_ref(), &sample, &t).pass);
        t.files.insert("notes.txt".into(), "hello evals".into());
        assert!(score_of(file_expectations().as_ref(), &sample, &t).pass);
    }

    #[test]
    fn tool_budget_bounds_both_sides() {
        let zero = Sample::new("a", "x").meta("max_tool_calls", 0);
        assert!(score_of(tool_call_budget().as_ref(), &zero, &transcript("4", &[])).pass);
        assert!(
            !score_of(
                tool_call_budget().as_ref(),
                &zero,
                &transcript("4", &["bash"])
            )
            .pass
        );
        let at_least = Sample::new("a", "x").meta("min_tool_calls", 1);
        assert!(!score_of(tool_call_budget().as_ref(), &at_least, &transcript("", &[])).pass);
        assert!(
            score_of(
                tool_call_budget().as_ref(),
                &at_least,
                &transcript("", &["read_file"])
            )
            .pass
        );
    }
}
