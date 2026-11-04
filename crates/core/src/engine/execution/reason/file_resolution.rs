//! File attachment resolution for the reason step.

use std::collections::HashMap;

use uuid::Uuid;

use super::RuntimeMessage;
use crate::file_services::{FileResolver, ResolvedFile};
use crate::image_services::ResolvedImage;

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

impl super::ReasonAtom {
    /// Resolve image_file references to actual image data
    ///
    /// This method extracts all image_file IDs from the messages and resolves
    /// them to base64-encoded image data using the configured ImageResolver.
    ///
    /// # Returns
    ///
    /// A HashMap mapping image IDs to ResolvedImage data. If no ImageResolver
    /// is configured, or if resolution fails for some images, those images
    /// will simply be missing from the map (and converted to placeholder text).
    pub(super) async fn resolve_images(
        &self,
        messages: &[RuntimeMessage],
    ) -> HashMap<Uuid, ResolvedImage> {
        let mut resolved = HashMap::new();

        // Check if we have an image resolver
        let resolver = match &self.image_resolver {
            Some(r) => r,
            None => return resolved,
        };

        // Collect all unique image_file IDs from all messages
        let image_ids: Vec<Uuid> = messages
            .iter()
            .flat_map(crate::engine::llm_conversions::extract_image_file_ids)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        if image_ids.is_empty() {
            return resolved;
        }

        tracing::debug!(
            image_count = image_ids.len(),
            "ReasonAtom: resolving image_file references"
        );

        // One batch: a turn can reference many screenshots, and a remote
        // worker would otherwise pay a round trip per image.
        match resolver.resolve_images(&image_ids).await {
            Ok(images) => {
                for image_id in &image_ids {
                    if !images.contains_key(image_id) {
                        tracing::warn!(
                            image_id = %image_id,
                            "ReasonAtom: image not found during resolution"
                        );
                    }
                }
                resolved = images;
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "ReasonAtom: failed to resolve images"
                );
            }
        }

        tracing::debug!(
            resolved_count = resolved.len(),
            "ReasonAtom: image resolution complete"
        );

        resolved
    }

    pub(super) async fn resolve_files(
        &self,
        messages: &[RuntimeMessage],
    ) -> HashMap<Uuid, ResolvedFile> {
        let Some(resolver) = &self.file_resolver else {
            return HashMap::new();
        };

        let file_ids: Vec<Uuid> = messages
            .iter()
            .flat_map(crate::engine::llm_conversions::extract_file_ids)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        if file_ids.is_empty() {
            return HashMap::new();
        }

        resolve_files_one_by_one(resolver.as_ref(), file_ids).await
    }
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
