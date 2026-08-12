//! Data Knowledge Capability
//!
//! Mounts a `/knowledge/` scaffold in the session filesystem for curated
//! data context: table docs, business rules, and validated query patterns.
//! Used by the Data Analyst harness to ground SQL generation in organizational
//! knowledge (layers 1-3 of the six-layer context pattern).
//!
//! The scaffold is shaped as an Open Knowledge Format (OKF) bundle: a directory
//! tree of markdown files with YAML frontmatter, navigated via `index.md`
//! files. Concept documents added under `tables/`, `business/`, and `queries/`
//! should carry frontmatter with at least a `type` field, so the same content
//! is portable to any OKF consumer. See knowledge/runtime-resources/okf-adoption.md.

use super::{
    Capability, CapabilityLocalization, CapabilityStatus, MountDirectoryBuilder, MountPoint,
};

pub const DATA_KNOWLEDGE_CAPABILITY_ID: &str = "data_knowledge";

pub struct DataKnowledgeCapability;

impl DataKnowledgeCapability {
    /// Root OKF index. `index.md` is the one OKF file permitted to carry
    /// frontmatter, used to declare the bundle's format version.
    const ROOT_INDEX: &'static str = "\
---
okf_version: \"0.1\"
type: Index
---
# Data Knowledge

This directory is an Open Knowledge Format (OKF) bundle: curated context for
data analysis, as markdown files with YAML frontmatter. See knowledge/runtime-resources/okf-adoption.md.

- [Tables & data sources](/tables/index.md)
- [Business rules & metrics](/business/index.md)
- [Validated query patterns](/queries/index.md)
";

    const TABLES_INDEX: &'static str = "\
# Table Documentation

Add one OKF concept document per table or data source. Each file needs YAML
frontmatter with at least a `type` (e.g. `type: Table`), plus a `title` and an
optional `resource` URI. The body should include:

- Column names and types
- Primary/foreign key relationships
- Known gotchas (NULLs, enums, timezone handling)
- Freshness: how often the data updates

Example: `orders.md`, `customers.md`. The data analyst agent reads these files
before writing SQL.
";

    const BUSINESS_INDEX: &'static str = "\
# Business Rules & Metric Definitions

Add OKF concept documents with organizational knowledge (frontmatter `type`,
e.g. `type: Metric` or `type: Business Definition`):

- Metric definitions (e.g. \"active user\" = logged in within 30 days)
- Business rules (e.g. revenue = net of refunds, not gross)
- Domain-specific terminology
- KPI calculation methods

The data analyst agent checks these before interpreting results.
";

    const QUERIES_INDEX: &'static str = "\
# Validated Query Patterns

Add OKF concept documents for known-good queries (frontmatter `type: Query`),
with the SQL in a fenced code block. For each:

- Explain what the query does
- Mark the expected grain (one row per customer, per day, etc.)
- Note any filters or assumptions

The data analyst agent uses these as templates for similar questions.
";
}

impl Capability for DataKnowledgeCapability {
    fn id(&self) -> &str {
        DATA_KNOWLEDGE_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Data Knowledge"
    }

    fn description(&self) -> &str {
        "Mounts a `/knowledge/` Open Knowledge Format (OKF) bundle with directories for table docs, business rules, and validated SQL patterns. Provides curated ground truth for data analysis, portable to any OKF consumer."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Знання про дані",
            "Монтує каркас `/knowledge/` у форматі Open Knowledge Format (OKF) з каталогами для документації таблиць, бізнес-правил і перевірених SQL-шаблонів. Надає кураторське достовірне джерело для аналізу даних.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("book-open")
    }

    fn category(&self) -> Option<&str> {
        Some("Data")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "Curated data knowledge is mounted at `/knowledge/{tables,business,queries}` as an Open Knowledge Format (OKF) bundle: markdown files with YAML frontmatter, navigable via `index.md` files. Read it before writing SQL; it is ground truth for schema semantics, metrics, and validated query patterns.",
        )
    }

    fn mounts(&self) -> Vec<MountPoint> {
        let knowledge_dir = MountDirectoryBuilder::new()
            .file("index.md", Self::ROOT_INDEX)
            .dir(
                "tables",
                MountDirectoryBuilder::new().file("index.md", Self::TABLES_INDEX),
            )
            .dir(
                "business",
                MountDirectoryBuilder::new().file("index.md", Self::BUSINESS_INDEX),
            )
            .dir(
                "queries",
                MountDirectoryBuilder::new().file("index.md", Self::QUERIES_INDEX),
            )
            .build();

        vec![MountPoint::readonly("/knowledge", knowledge_dir, self.id())]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_file_system"]
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["file_system"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::capability_types::MountSource;

    // Metadata constants covered by builtin_capabilities_satisfy_registry_invariants.

    #[test]
    fn test_has_system_prompt() {
        let cap = DataKnowledgeCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("/knowledge/{tables,business,queries}"));
        assert!(prompt.contains("ground truth"));
        // Framed as an OKF bundle so agents navigate it accordingly.
        assert!(prompt.contains("Open Knowledge Format"));
    }

    #[test]
    fn test_mounts_knowledge_scaffold() {
        let cap = DataKnowledgeCapability;
        let mounts = cap.mounts();
        assert_eq!(mounts.len(), 1);

        let mount = &mounts[0];
        assert_eq!(mount.path, "/knowledge");
        assert!(mount.is_readonly());
        assert_eq!(mount.capability_id, "data_knowledge");

        match &mount.source {
            MountSource::InlineDirectory { entries } => {
                // Three concept directories plus the OKF root index.md.
                assert_eq!(entries.len(), 4);
                assert!(entries.contains_key("tables"));
                assert!(entries.contains_key("business"));
                assert!(entries.contains_key("queries"));
                assert!(entries.contains_key("index.md"));
            }
            _ => panic!("Expected InlineDirectory"),
        }
    }

    #[test]
    fn test_root_index_declares_okf_version() {
        let cap = DataKnowledgeCapability;
        let mounts = cap.mounts();
        match &mounts[0].source {
            MountSource::InlineDirectory { entries } => {
                match entries.get("index.md").map(|e| &e.source) {
                    Some(MountSource::InlineFile { content, .. }) => {
                        assert!(content.contains("okf_version: \"0.1\""));
                    }
                    other => panic!("expected index.md inline file, got {other:?}"),
                }
            }
            _ => panic!("Expected InlineDirectory"),
        }
    }
}
