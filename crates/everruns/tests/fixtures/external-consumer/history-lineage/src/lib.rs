//! Downstream consumer fixture for conversation branch import (EVE-1305).
//!
//! The product owns its saved format: an append-only event log that keeps
//! abandoned suffixes, and a separate timeline naming each branch's active
//! sequence ranges. Everruns never sees that format. The product parses it
//! into [`ConversationLineage`] and hands it to the cutover, so the authority
//! owns branch selection from then on.

use everruns::hosting::{ConversationLineage, HistoryBranch, SequenceRange};

/// Parse a product timeline: `branch <id> <first>-<last>[,<first>-<last>...]`
/// lines and one `active <id>` line. Blank lines and `#` comments are ignored.
pub fn parse_timeline(text: &str) -> Result<ConversationLineage, String> {
    let mut branches = Vec::new();
    let mut active = None;
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut words = line.split_whitespace();
        match (words.next(), words.next(), words.next()) {
            (Some("active"), Some(id), None) => active = Some(id.to_owned()),
            (Some("branch"), Some(id), ranges) => branches.push(HistoryBranch {
                id: id.to_owned(),
                ranges: ranges
                    .unwrap_or_default()
                    .split(',')
                    .filter(|range| !range.is_empty())
                    .map(parse_range)
                    .collect::<Result<_, _>>()?,
            }),
            _ => return Err(format!("unrecognized timeline line: {line}")),
        }
    }
    let active = active.ok_or("timeline names no active branch")?;
    Ok(ConversationLineage::new(active, branches))
}

fn parse_range(range: &str) -> Result<SequenceRange, String> {
    let (first, last) = range
        .split_once('-')
        .ok_or_else(|| format!("invalid range {range}"))?;
    let parse = |value: &str| {
        value
            .parse::<i32>()
            .map_err(|error| format!("invalid range {range}: {error}"))
    };
    Ok(SequenceRange::new(parse(first)?, parse(last)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns::hosting::spi::{EventReadLimit, EventReadRequest, HostBackends};
    use everruns::hosting::{AgentBuild, CutoverOptions, SqliteHosting};
    use everruns::{Agent, Engine, Model};

    fn agent() -> Agent {
        Agent::builder()
            .instructions("Answer briefly.")
            .model(Model::simulated("Hello!"))
            .build()
            .unwrap()
    }

    // The saved product session is the legacy source the cutover moves.
    #[allow(deprecated)]
    #[tokio::test]
    async fn product_timeline_imports_as_authority_owned_branches() {
        // The product's saved session: two turns, then a rewind to the first.
        let backends = HostBackends::in_memory();
        let legacy = Agent::builder()
            .instructions("Answer briefly.")
            .model(Model::simulated("Hello!"))
            .backends(backends.clone())
            .build()
            .unwrap();
        let saved = Engine::new().create(legacy.clone());
        let id = saved.session_id();
        assert!(saved.run("alpha").await.unwrap().success);
        assert!(saved.run("beta").await.unwrap().success);
        drop(saved);
        let page = backends
            .event_log
            .read_page(EventReadRequest::new(
                id,
                EventReadLimit::new(1000).unwrap(),
            ))
            .await
            .unwrap();
        // The product's undo points are turn ends: the last event of a turn.
        let mut ends: Vec<(String, i32)> = Vec::new();
        for event in &page.events {
            let (Some(turn), Some(sequence)) = (event.context.turn_id, event.sequence) else {
                continue;
            };
            let turn = turn.to_string();
            match ends.iter_mut().find(|(id, _)| *id == turn) {
                Some(end) => end.1 = sequence,
                None => ends.push((turn, sequence)),
            }
        }
        let ends: Vec<i32> = ends.into_iter().map(|(_, end)| end).collect();
        let timeline = format!(
            "# product timeline\nbranch main 1-{}\nbranch undo-1 1-{}\nactive undo-1\n",
            ends[1], ends[0]
        );
        let lineage = parse_timeline(&timeline).unwrap();

        let directory = tempfile::tempdir().unwrap();
        let profile = SqliteHosting::open(directory.path().join("authority.db")).unwrap();
        let build = AgentBuild::new("assistant", "imported-v1");
        let engine = Engine::builder()
            .hosting(profile.clone())
            .register(build.clone(), agent())
            .build();
        let options = CutoverOptions::new().lineage(lineage);
        let report = engine
            .cut_over_legacy_session_with_options(id, &legacy, &build, options.clone())
            .await
            .unwrap();
        assert_eq!(report.branches, 2);
        assert!(
            engine
                .cut_over_legacy_session_with_options(id, &legacy, &build, options)
                .await
                .unwrap()
                .replayed
        );

        let session = engine.resume(id).await.unwrap();
        let texts = |page: everruns::HistoryPage| {
            page.messages
                .iter()
                .map(|message| message.text())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            texts(session.history().page().await.unwrap()),
            ["alpha", "Hello!"]
        );
        let sent = session.send("gamma").await.unwrap();
        assert!(sent.wait().await.unwrap().success);
        assert_eq!(
            texts(session.history().page().await.unwrap()),
            ["alpha", "Hello!", "gamma", "Hello!"]
        );
        // The raw export still holds the abandoned turn with its identities.
        let mut raw = Vec::new();
        let exported = profile.export_jsonl(id, &mut raw).unwrap();
        assert!(exported > page.events.len());
        let raw = String::from_utf8(raw).unwrap();
        assert!(
            page.events
                .iter()
                .all(|event| raw.contains(&event.id.to_string()))
        );

        // Redo through the authority, never through a downstream filter.
        let lineage = engine.history_lineage(id).await.unwrap();
        assert_eq!(lineage.active, "undo-1");
        engine
            .select_history_branch(id, uuid::Uuid::new_v4(), lineage.revision, "main", None)
            .await
            .unwrap();
        assert_eq!(
            texts(session.history().page().await.unwrap()),
            ["alpha", "Hello!", "beta", "Hello!"]
        );
    }

    #[test]
    fn malformed_timelines_are_refused_by_the_product_parser() {
        assert!(parse_timeline("branch main 1-4\n").is_err());
        assert!(parse_timeline("branch main 1:4\nactive main\n").is_err());
        assert_eq!(
            parse_timeline("branch main\nactive main\n")
                .unwrap()
                .branches[0]
                .ranges,
            []
        );
    }
}
