//! Data Knowledge Capability
//!
//! Mounts a `/knowledge/` scaffold in the session filesystem for curated
//! data context: table docs, business rules, and validated query patterns.
//! Used by the Data Analyst harness to ground SQL generation in organizational
//! knowledge (layers 1-3 of the six-layer context pattern).

use super::{Capability, CapabilityStatus, MountDirectoryBuilder, MountPoint};

pub struct DataKnowledgeCapability;

impl DataKnowledgeCapability {
    const TABLES_README: &'static str = "\
# Table Documentation

Add one markdown file per table or data source. Include:
- Column names and types
- Primary/foreign key relationships
- Known gotchas (NULLs, enums, timezone handling)
- Freshness: how often the data updates
- Example: `orders.md`, `customers.md`

The data analyst agent reads these files before writing SQL.
";

    const BUSINESS_README: &'static str = "\
# Business Rules & Metric Definitions

Add markdown files with organizational knowledge:
- Metric definitions (e.g. \"active user\" = logged in within 30 days)
- Business rules (e.g. revenue = net of refunds, not gross)
- Domain-specific terminology
- KPI calculation methods

The data analyst agent checks these before interpreting results.
";

    const QUERIES_README: &'static str = "\
# Validated Query Patterns

Add `.sql` files with known-good queries:
- Include comments explaining what each query does
- Mark the expected grain (one row per customer, per day, etc.)
- Note any filters or assumptions

The data analyst agent uses these as templates for similar questions.
";
}

impl Capability for DataKnowledgeCapability {
    fn id(&self) -> &str {
        "data_knowledge"
    }

    fn name(&self) -> &str {
        "Data Knowledge"
    }

    fn description(&self) -> &str {
        "Mounts a `/knowledge/` scaffold with directories for table docs, business rules, and validated SQL patterns. Provides curated ground truth for data analysis."
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
            "Curated data knowledge is available in /knowledge/:\n\
             - /knowledge/tables/ — schema docs, column semantics, gotchas\n\
             - /knowledge/business/ — metric definitions, business rules\n\
             - /knowledge/queries/ — validated SQL templates\n\
             \n\
             Read these files before writing SQL. They are curated ground truth.",
        )
    }

    fn mounts(&self) -> Vec<MountPoint> {
        let knowledge_dir = MountDirectoryBuilder::new()
            .dir(
                "tables",
                MountDirectoryBuilder::new().file("README.md", Self::TABLES_README),
            )
            .dir(
                "business",
                MountDirectoryBuilder::new().file("README.md", Self::BUSINESS_README),
            )
            .dir(
                "queries",
                MountDirectoryBuilder::new().file("README.md", Self::QUERIES_README),
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
    use crate::capability_types::MountSource;

    #[test]
    fn test_capability_metadata() {
        let cap = DataKnowledgeCapability;
        assert_eq!(cap.id(), "data_knowledge");
        assert_eq!(cap.name(), "Data Knowledge");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("book-open"));
        assert_eq!(cap.category(), Some("Data"));
    }

    #[test]
    fn test_has_system_prompt() {
        let cap = DataKnowledgeCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("/knowledge/tables/"));
        assert!(prompt.contains("/knowledge/business/"));
        assert!(prompt.contains("/knowledge/queries/"));
    }

    #[test]
    fn test_has_no_tools() {
        let cap = DataKnowledgeCapability;
        assert!(cap.tools().is_empty());
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
                assert_eq!(entries.len(), 3);
                assert!(entries.contains_key("tables"));
                assert!(entries.contains_key("business"));
                assert!(entries.contains_key("queries"));
            }
            _ => panic!("Expected InlineDirectory"),
        }
    }

    #[test]
    fn test_depends_on_file_system() {
        let cap = DataKnowledgeCapability;
        assert_eq!(cap.dependencies(), vec!["session_file_system"]);
    }
}
