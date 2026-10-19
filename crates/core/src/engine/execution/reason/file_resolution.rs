//! File attachment resolution for the reason step.

use std::collections::HashMap;

use uuid::Uuid;

use crate::file_services::{FileResolver, ResolvedFile};

/// Resolve files with one resolver call each.
///
/// Decision: a batch is all-or-nothing, so one oversized or missing file (or a
/// server-side batch cap) used to turn every file in the window into a
/// placeholder. Per-file calls keep each failure to its own file.
// Design: knowledge/runtime-resources/session-attachments.md.
pub(crate) async fn resolve_files_one_by_one(
    resolver: &dyn FileResolver,
    file_ids: Vec<Uuid>,
) -> HashMap<Uuid, ResolvedFile> {
    let mut resolved = HashMap::new();
    for file_id in file_ids {
        match resolver.resolve_files(&[file_id]).await {
            Ok(map) => resolved.extend(map),
            Err(e) => {
                tracing::warn!(
                    target: "reason",
                    file_id = %file_id,
                    "ReasonAtom: file resolution failed: {e}"
                );
            }
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A resolver that fails any batch of more than one file, like the server's
    /// per-resolution cap, and fails one specific file outright.
    struct SingleFileResolver {
        broken: Uuid,
    }

    #[async_trait::async_trait]
    impl crate::file_services::FileResolver for SingleFileResolver {
        async fn resolve_files(
            &self,
            ids: &[Uuid],
        ) -> everruns_contracts::error::Result<HashMap<Uuid, crate::file_services::ResolvedFile>>
        {
            if ids.len() > 1 {
                return Err(everruns_contracts::error::AgentLoopError::store(
                    "batch too large",
                ));
            }
            if ids.contains(&self.broken) {
                return Err(everruns_contracts::error::AgentLoopError::store(
                    "file too large",
                ));
            }
            Ok(ids
                .iter()
                .map(|id| {
                    (
                        *id,
                        crate::file_services::ResolvedFile {
                            base64: "JVBERi0=".to_string(),
                            media_type: "application/pdf".to_string(),
                            filename: Some(format!("{id}.pdf")),
                        },
                    )
                })
                .collect())
        }
    }

    #[tokio::test]
    async fn one_failing_file_does_not_drop_the_others() {
        let ok_a = Uuid::now_v7();
        let ok_b = Uuid::now_v7();
        let broken = Uuid::now_v7();
        let resolver = SingleFileResolver { broken };

        let resolved = resolve_files_one_by_one(&resolver, vec![ok_a, broken, ok_b]).await;

        assert_eq!(resolved.len(), 2);
        assert!(resolved.contains_key(&ok_a));
        assert!(resolved.contains_key(&ok_b));
        assert!(!resolved.contains_key(&broken));
    }
}
