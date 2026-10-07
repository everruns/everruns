//! Manager context rows (`entity_manager_context`): one markdown document per
//! entity. See `crate::domains::change_history::context` for who may read and
//! write them; the storage only reads, writes with a revision check, and
//! deletes.

use chrono::{DateTime, Utc};
use everruns_server_macros::Columns;
use uuid::Uuid;

/// Largest manager context document, in bytes.
pub const MAX_MANAGER_CONTEXT_BYTES: usize = 16 * 1024;

/// The stored document for one entity.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow, Columns)]
pub struct ManagerContextRow {
    pub org_id: i64,
    pub entity_kind: String,
    pub entity_ref: String,
    pub content: String,
    pub revision: i64,
    pub updated_by_user_id: Option<Uuid>,
    pub updated_at: DateTime<Utc>,
}

/// Which entity's document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagerContextKey {
    pub org_id: i64,
    pub entity_kind: String,
    pub entity_ref: String,
}

/// A write to a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManagerContextEdit {
    /// Replace the whole document.
    Set(String),
    /// Add a paragraph at the end.
    Append(String),
    /// Empty the document (the row and its revision stay).
    Clear,
}

impl ManagerContextEdit {
    /// The document after this edit, from `current` (empty when unset).
    pub fn apply(&self, current: &str) -> String {
        match self {
            Self::Set(content) => content.clone(),
            Self::Append(text) if current.trim().is_empty() => text.clone(),
            Self::Append(text) => format!("{}\n\n{text}", current.trim_end()),
            Self::Clear => String::new(),
        }
    }
}

/// Why a write was refused.
#[derive(Debug, thiserror::Error)]
pub enum ManagerContextWriteError {
    /// The writer read an older revision than the stored one.
    #[error("manager context is at revision {current}, not {expected}")]
    Stale { expected: i64, current: i64 },
    /// The document would exceed `MAX_MANAGER_CONTEXT_BYTES`.
    #[error("manager context would be {0} bytes; the limit is {MAX_MANAGER_CONTEXT_BYTES}")]
    TooLarge(usize),
    #[error(transparent)]
    Storage(#[from] anyhow::Error),
}

/// Check a write against the stored revision (0 when no document exists) and
/// compute the new document. Both stores run this inside their write lock or
/// transaction, so it states the write's meaning once.
pub fn plan_write(
    current: Option<&ManagerContextRow>,
    edit: &ManagerContextEdit,
    expected_revision: Option<i64>,
) -> Result<(String, i64), ManagerContextWriteError> {
    let current_revision = current.map_or(0, |row| row.revision);
    if let Some(expected) = expected_revision
        && expected != current_revision
    {
        return Err(ManagerContextWriteError::Stale {
            expected,
            current: current_revision,
        });
    }
    let content = edit.apply(current.map_or("", |row| row.content.as_str()));
    if content.len() > MAX_MANAGER_CONTEXT_BYTES {
        return Err(ManagerContextWriteError::TooLarge(content.len()));
    }
    Ok((content, current_revision + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(content: &str, revision: i64) -> ManagerContextRow {
        ManagerContextRow {
            org_id: 1,
            entity_kind: "agent".into(),
            entity_ref: "agent_1".into(),
            content: content.into(),
            revision,
            updated_by_user_id: None,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn append_adds_a_paragraph_and_set_replaces() {
        let current = row("Owned by support.\n", 3);
        let (content, revision) = plan_write(
            Some(&current),
            &ManagerContextEdit::Append("Keep it kid friendly.".into()),
            None,
        )
        .unwrap();
        assert_eq!(content, "Owned by support.\n\nKeep it kid friendly.");
        assert_eq!(revision, 4);
        let (content, _) =
            plan_write(None, &ManagerContextEdit::Append("first".into()), None).unwrap();
        assert_eq!(content, "first");
        let (content, _) =
            plan_write(Some(&current), &ManagerContextEdit::Set("new".into()), None).unwrap();
        assert_eq!(content, "new");
        let (content, _) = plan_write(Some(&current), &ManagerContextEdit::Clear, None).unwrap();
        assert_eq!(content, "");
    }

    #[test]
    fn a_stale_revision_or_an_oversized_document_is_refused() {
        let current = row("x", 2);
        assert!(matches!(
            plan_write(Some(&current), &ManagerContextEdit::Clear, Some(1)),
            Err(ManagerContextWriteError::Stale {
                expected: 1,
                current: 2
            })
        ));
        // No document yet is revision 0.
        assert!(plan_write(None, &ManagerContextEdit::Set("a".into()), Some(0)).is_ok());
        let huge = "a".repeat(MAX_MANAGER_CONTEXT_BYTES + 1);
        assert!(matches!(
            plan_write(None, &ManagerContextEdit::Set(huge), None),
            Err(ManagerContextWriteError::TooLarge(_))
        ));
    }
}
