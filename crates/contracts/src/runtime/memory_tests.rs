use super::*;
use chrono::TimeZone;

fn at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 26, 8, 0, 0).unwrap()
}

fn new(kind: MemoryEntryKind, subject: Option<&str>) -> NewMemoryEntry {
    NewMemoryEntry {
        kind,
        subject: subject.map(str::to_string),
        statement: "Allowlist @a2ui/web_core".into(),
        quote: None,
        source: None,
        occurred_at: None,
    }
}

fn entry(kind: MemoryEntryKind, subject: Option<&str>, statement: &str) -> MemoryEntry {
    let mut draft = new(kind, subject);
    draft.statement = statement.into();
    MemoryEntry {
        store: MemoryStoreKey::scope(MemoryScope::Agent),
        path: entry_path(&draft, at(), false),
        kind,
        subject: subject.map(str::to_string),
        statement: statement.into(),
        quote: None,
        source: None,
        occurred_at: at(),
        recorded_at: at(),
        recorded_by: "agent:test".into(),
        resolved_at: None,
        resolution: None,
        proposed: false,
    }
}

#[test]
fn subjects_match_case_and_space_insensitively() {
    assert_eq!(
        normalize_subject("  Everruns#5528   @a2ui ").unwrap(),
        "everruns#5528 @a2ui"
    );
    assert!(normalize_subject("   ").is_err());
    assert!(normalize_subject(&"x".repeat(MAX_SUBJECT_CHARS + 1)).is_err());
}

#[test]
fn every_kind_but_note_needs_a_subject() {
    for kind in MemoryEntryKind::ALL {
        let result = validate_new_entry(new(kind, None));
        assert_eq!(result.is_ok(), kind == MemoryEntryKind::Note, "{kind:?}");
    }
    let ok = validate_new_entry(new(MemoryEntryKind::Decision, Some(" Everruns#5528 "))).unwrap();
    assert_eq!(ok.subject.as_deref(), Some("everruns#5528"));
}

#[test]
fn validation_rejects_empty_long_and_bad_sources() {
    let mut empty = new(MemoryEntryKind::Note, None);
    empty.statement = "  ".into();
    assert!(validate_new_entry(empty).is_err());

    let mut long = new(MemoryEntryKind::Note, None);
    long.statement = "x".repeat(MAX_STATEMENT_CHARS + 1);
    assert!(validate_new_entry(long).is_err());

    let mut bad_kind = new(MemoryEntryKind::Note, None);
    bad_kind.source = Some(MemorySource {
        kind: "email".into(),
        ..Default::default()
    });
    assert!(validate_new_entry(bad_kind).is_err());

    let mut bad_url = new(MemoryEntryKind::Note, None);
    bad_url.source = Some(MemorySource {
        kind: "github".into(),
        reference: Some("everruns/everruns#5528".into()),
        url: Some("javascript:alert(1)".into()),
    });
    assert!(validate_new_entry(bad_url).is_err());

    let mut good = new(MemoryEntryKind::Note, None);
    good.source = Some(MemorySource {
        kind: " Slack ".into(),
        reference: Some("  ".into()),
        url: Some("https://slack.com/x".into()),
    });
    let source = validate_new_entry(good).unwrap().source.unwrap();
    assert_eq!(source.kind, "slack");
    assert_eq!(source.reference, None);
}

#[test]
fn slugs_and_entry_paths() {
    assert_eq!(
        slug("everruns#5528 @a2ui/web_core"),
        "everruns-5528-a2ui-web-core"
    );
    assert_eq!(slug("!!!"), "entry");
    assert!(slug(&"ab ".repeat(100)).len() <= MAX_SLUG_CHARS);

    let decision = new(MemoryEntryKind::Decision, Some("everruns#5528"));
    assert_eq!(
        entry_path(&decision, at(), false),
        "/decisions/everruns-5528.md"
    );
    assert_eq!(
        entry_path(&decision, at(), true),
        "/proposals/decisions/everruns-5528.md"
    );
    let note = new(MemoryEntryKind::Note, Some("merges"));
    assert_eq!(
        entry_path(&note, at(), false),
        "/notes/2026-11-26-merges.md"
    );
    let loose = new(MemoryEntryKind::Note, None);
    assert_eq!(
        entry_path(&loose, at(), false),
        "/notes/2026-11-26-080000-allowlist-a2ui-web-core.md"
    );
}

#[test]
fn entry_locations() {
    assert_eq!(
        entry_location("/decisions/x.md"),
        Some((MemoryEntryKind::Decision, false))
    );
    assert_eq!(
        entry_location("/proposals/rules/x.md"),
        Some((MemoryEntryKind::Rule, true))
    );
    for not_entry in [
        "/notes.md",
        "/decisions/x.txt",
        "/decisions/sub/x.md",
        "/random/x.md",
        "/decisions/.md",
        "/proposals/x.md",
    ] {
        assert_eq!(entry_location(not_entry), None, "{not_entry}");
    }
}

#[test]
fn entry_files_round_trip() {
    let mut original = entry(
        MemoryEntryKind::Blocked,
        Some("everruns#5528"),
        "Waiting on Mike to allowlist @a2ui/web_core.\nSecond line.",
    );
    original.quote = Some("allow it\nfor now".into());
    original.source = Some(MemorySource {
        kind: "github".into(),
        reference: Some("everruns/everruns#5528".into()),
        url: Some("https://github.com/everruns/everruns/pull/5528".into()),
    });
    original.resolved_at = Some(at());
    original.resolution = Some("allowlisted".into());
    let text = render_entry_file(&original);
    assert!(text.starts_with("---\nkind: blocked\nsubject: everruns#5528\nstatus: resolved\n"));
    assert!(text.contains("\n> allow it\n> for now\n"));
    let parsed = parse_entry_file(&original.store, &original.path, &text).unwrap();
    assert_eq!(parsed, original);
}

#[test]
fn hand_written_entry_files_parse_with_defaults() {
    let store = MemoryStoreKey::named("everruns-maintenance");
    let text = "---\nsubject: Merges\noccurred_at: 2026-11-20\n---\nSquash merge only.\n";
    let parsed = parse_entry_file(&store, "/proposals/rules/merges.md", text).unwrap();
    assert_eq!(parsed.kind, MemoryEntryKind::Rule);
    assert_eq!(parsed.subject.as_deref(), Some("merges"));
    assert_eq!(parsed.statement, "Squash merge only.");
    assert_eq!(parsed.recorded_by, "person");
    assert!(parsed.proposed);
    assert!(!parsed.is_open());
    assert_eq!(
        parsed.session_path(),
        "/memory/everruns-maintenance/proposals/rules/merges.md"
    );
}

#[test]
fn bad_entry_files_say_what_is_wrong() {
    let store = MemoryStoreKey::scope(MemoryScope::Agent);
    let cases = [
        ("/decisions/x.md", "no header\n", "starts with"),
        ("/decisions/x.md", "---\nsubject: x\nbody\n", "closing"),
        (
            "/decisions/x.md",
            "---\nsubject: x\ncolour: red\n---\nb\n",
            "unknown header",
        ),
        (
            "/decisions/x.md",
            "---\nkind: rule\nsubject: x\n---\nb\n",
            "lives in /rules/",
        ),
        (
            "/decisions/x.md",
            "---\nsubject: x\noccurred_at: soon\n---\nb\n",
            "not a date",
        ),
        (
            "/decisions/x.md",
            "---\nsubject: x\nstatus: done\n---\nb\n",
            "status",
        ),
        (
            "/decisions/x.md",
            "---\nsubject: x\nstatus: resolved\n---\nb\n",
            "only a blocked",
        ),
        ("/decisions/x.md", "---\n---\nb\n", "subject"),
        ("/notes.md", "---\n---\nb\n", "not an entry path"),
    ];
    for (path, text, expected) in cases {
        let error = match parse_entry_file(&store, path, text) {
            Ok(_) => panic!("{text:?} parsed"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains(expected), "{text:?}: {error}");
    }
}

#[test]
fn query_terms_keep_identifiers() {
    assert_eq!(
        query_terms("What did we decide about everruns#5528 and @a2ui/web_core?"),
        vec!["decide", "everruns#5528", "a2ui", "web_core"]
    );
    assert!(query_terms("the a of").is_empty());
}

#[test]
fn lexical_score_prefers_subject_matches() {
    let terms = query_terms("5528 allowlist");
    let by_subject = lexical_score(&terms, Some("everruns#5528"), "allowlist it");
    let by_text = lexical_score(&terms, Some("other"), "5528 allowlist it");
    assert!(by_subject > by_text);
    assert_eq!(lexical_score(&terms, None, "nothing"), 0.0);
}

#[test]
fn file_titles_and_excerpts() {
    assert_eq!(
        file_title("\n# Decisions\nmore").as_deref(),
        Some("Decisions")
    );
    assert_eq!(file_title("plain line").as_deref(), Some("plain line"));
    assert_eq!(file_title("   \n"), None);
    let content = "a\nb\n#5528 allowlisted\nc\nd";
    assert_eq!(
        excerpt(content, &query_terms("#5528")).as_deref(),
        Some("b\n#5528 allowlisted\nc")
    );
}

#[test]
fn card_lists_blocked_first_and_stays_in_budget() {
    let mut entries = vec![
        entry(MemoryEntryKind::Decision, Some("a"), "decided"),
        entry(MemoryEntryKind::Blocked, Some("b"), "waiting on a person"),
    ];
    card_order(&mut entries);
    assert_eq!(entries[0].kind, MemoryEntryKind::Blocked);

    let card = MemoryCard {
        stores: vec![
            MemoryStoreSummary {
                store: MemoryStoreKey::scope(MemoryScope::Agent),
                writable: true,
                files_mounted: true,
                description: None,
                entries,
                more_entries: 0,
                files: vec![MemoryFileSummary {
                    path: "/notes.md".into(),
                    title: Some("Notes".into()),
                }],
                more_files: 0,
            },
            MemoryStoreSummary {
                store: MemoryStoreKey::scope(MemoryScope::Team),
                writable: true,
                files_mounted: false,
                description: None,
                entries: vec![],
                more_entries: 0,
                files: vec![],
                more_files: 0,
            },
        ],
    };
    let text = card.render().unwrap();
    assert!(text.contains("## agent memory (read-write, files at /memory/agent)"));
    assert!(text.contains("- blocked [b]"));
    assert!(text.contains("/memory/agent/notes.md: Notes"));
    assert!(text.contains("## team memory (read-write): shared by your team"));
    assert!(text.contains("blocked (work waiting on someone"));
    assert!(text.contains("(empty)"));
    assert!(text.find("blocked").unwrap() < text.find("decision [a]").unwrap());

    let many = MemoryCard {
        stores: vec![MemoryStoreSummary {
            store: MemoryStoreKey::scope(MemoryScope::Org),
            writable: false,
            files_mounted: false,
            description: None,
            entries: (0..200)
                .map(|i| {
                    entry(
                        MemoryEntryKind::Rule,
                        Some(&format!("r{i}")),
                        &"x".repeat(250),
                    )
                })
                .collect(),
            more_entries: 5,
            files: vec![],
            more_files: 0,
        }],
    };
    let text = many.render().unwrap();
    assert!(text.len() < CARD_BUDGET_CHARS + 400);
    assert!(text.contains("more; `recall` with store org"));
    assert_eq!(MemoryCard::default().render(), None);
}

#[test]
fn store_keys_parse_scopes_and_names() {
    assert_eq!(
        MemoryStoreKey::parse("team").unwrap(),
        MemoryStoreKey::scope(MemoryScope::Team)
    );
    assert_eq!(
        MemoryStoreKey::scope(MemoryScope::Team).mount_path(),
        "/memory/team"
    );
    assert_eq!(
        MemoryStoreKey::parse("parent").unwrap(),
        MemoryStoreKey::scope(MemoryScope::Parent)
    );
    let named = MemoryStoreKey::parse("everruns-maintenance").unwrap();
    assert_eq!(named.scope, MemoryScope::Named);
    assert_eq!(named.mount_path(), "/memory/everruns-maintenance");
    for bad in ["named", "shared", "Upper", "-x", "x-", "a b", "", "9lives"] {
        assert!(MemoryStoreKey::parse(bad).is_err(), "{bad}");
    }
    assert!(!MemoryScope::Agent.shared());
    assert!(MemoryScope::Team.shared());
    assert!(MemoryScope::Named.shared());
}

#[test]
fn config_switches_each_store() {
    let defaults = AgentMemoryConfig::from_value(&serde_json::Value::Null).unwrap();
    assert_eq!(defaults, AgentMemoryConfig::default());
    let labels: Vec<_> = defaults
        .stores()
        .into_iter()
        .map(|(store, access)| (store.label(), access))
        .collect();
    assert_eq!(
        labels,
        vec![
            ("agent".to_string(), MemoryAccess::ReadWrite),
            ("team".to_string(), MemoryAccess::ReadWrite),
            ("parent".to_string(), MemoryAccess::Read),
            ("user".to_string(), MemoryAccess::ReadWrite),
            ("org".to_string(), MemoryAccess::Read),
        ]
    );

    let config = AgentMemoryConfig::from_value(&serde_json::json!({
        "user": "off",
        "parent": "off",
        "attach": [{"memory": "everruns-maintenance", "mode": "read_write"}, {"memory": "docs", "mode": "read"}]
    }))
    .unwrap();
    let named = MemoryStoreKey::named("everruns-maintenance");
    assert_eq!(config.access(&named), MemoryAccess::ReadWrite);
    assert_eq!(
        config.access(&MemoryStoreKey::named("other")),
        MemoryAccess::Off
    );
    assert_eq!(
        config.access(&MemoryStoreKey::named("docs")),
        MemoryAccess::Read
    );
    let labels: Vec<_> = config
        .stores()
        .into_iter()
        .map(|(s, _)| s.label())
        .collect();
    assert_eq!(
        labels,
        ["agent", "team", "org", "everruns-maintenance", "docs"]
    );

    assert!(
        AgentMemoryConfig::from_value(&serde_json::json!({"attach": [{"memory": "org"}]})).is_err()
    );
    assert!(AgentMemoryConfig::from_value(&serde_json::json!({"agent": "sometimes"})).is_err());
}
