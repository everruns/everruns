#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! The upstream AG-UI 1.0 client conformance corpus
//! (`spec/1.0/conformance/streams`), replayed through [`RunConsumer`].
//!
//! Every stream the corpus says a client consumes (`outcome: completed`)
//! must be accepted, and every one it says a client rejects
//! (`outcome: failed`) must be rejected, with an error containing the
//! corpus's `errorContains` text unless the wording is listed in
//! [`WORDING_DIFFERS`]. The corpus is pinned by its `MANIFEST.txt`: a
//! fixture added or removed upstream fails this test until it is reviewed.
//!
//! What is not checked, and why: the corpus also asserts warnings, delivered
//! event lists, assembled messages and state, and the outgoing request.
//! Those describe the first-party TypeScript client's reducers (it keeps
//! state, activity messages and a message list; this crate assembles a
//! narrower [`everruns_ag_ui::consumer::RunResult`]), and the corpus README
//! says they need relaxing for any other client. Accept and reject is the
//! conformance contract.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use everruns_ag_ui::consumer::{ProtocolError, RunConsumer};
use serde_json::Value;

/// Fixtures whose rejection message is worded differently here. The stream is
/// still rejected; only `errorContains` is not compared.
const WORDING_DIFFERS: &[(&str, &str)] = &[(
    "reasoning-message-wrong-role-fatal",
    "serde reports `expected `reasoning``, the TypeScript client's zod reports `expected \"reasoning\"`",
)];

/// Fixtures excluded from the accept/reject check entirely. Empty today;
/// an entry needs a reason, and the test fails if one goes stale.
const OUT_OF_SCOPE: &[(&str, &str)] = &[];

fn streams_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("spec/1.0/conformance/streams")
}

fn manifest() -> BTreeSet<String> {
    fs::read_to_string(streams_dir().join("MANIFEST.txt"))
        .unwrap()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

fn replay(stream: &[Value]) -> Result<(), ProtocolError> {
    let mut consumer = RunConsumer::new();
    for event in stream {
        consumer.push_value(event.clone())?;
    }
    consumer.finish().map(|_| ())
}

#[test]
fn corpus_listing_matches_manifest() {
    let on_disk: BTreeSet<String> = fs::read_dir(streams_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".json"))
        .collect();
    assert_eq!(on_disk, manifest(), "MANIFEST.txt and streams/ disagree");
    assert!(
        on_disk.len() >= 60,
        "the corpus shrank to {}",
        on_disk.len()
    );
}

#[test]
fn every_corpus_stream_is_accepted_or_rejected_as_specified() {
    let mut failures = Vec::new();
    let mut wording_used = BTreeSet::new();
    let mut scope_used = BTreeSet::new();

    for file in manifest() {
        let fixture: Value =
            serde_json::from_str(&fs::read_to_string(streams_dir().join(&file)).unwrap()).unwrap();
        let name = fixture["name"].as_str().unwrap().to_owned();
        assert_eq!(
            format!("{name}.json"),
            file,
            "fixture name must match its file"
        );
        if OUT_OF_SCOPE.iter().any(|(n, _)| *n == name) {
            scope_used.insert(name);
            continue;
        }

        let expect = &fixture["expect"];
        let stream = fixture["stream"].as_array().unwrap();
        let outcome = replay(stream);
        match expect["outcome"].as_str() {
            Some("completed") => {
                if let Err(err) = outcome {
                    failures.push(format!("{name}: must be accepted, rejected with: {err}"));
                }
            }
            Some("failed") => match outcome {
                Ok(()) => failures.push(format!("{name}: must be rejected, was accepted")),
                Err(err) => {
                    if let Some(needle) = expect["errorContains"].as_str()
                        && !err.message().contains(needle)
                    {
                        if WORDING_DIFFERS.iter().any(|(n, _)| *n == name) {
                            wording_used.insert(name);
                        } else {
                            failures.push(format!(
                                "{name}: rejected, but the error does not contain {needle:?}: {err}"
                            ));
                        }
                    }
                }
            },
            other => failures.push(format!("{name}: unknown expected outcome {other:?}")),
        }
    }

    for (name, _) in WORDING_DIFFERS {
        if !wording_used.contains(*name) {
            failures.push(format!(
                "{name}: listed in WORDING_DIFFERS but the wording now matches; remove it"
            ));
        }
    }
    for (name, _) in OUT_OF_SCOPE {
        if !scope_used.contains(*name) {
            failures.push(format!(
                "{name}: listed in OUT_OF_SCOPE but not in the corpus; remove it"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
