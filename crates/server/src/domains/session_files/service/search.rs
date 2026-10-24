//! Exact search across workspace and host-owned mounts.
use super::*;
impl WorkspaceFileService {
    pub async fn grep(&self, session_id: Uuid, req: GrepInput) -> Result<Vec<GrepResult>> {
        self.restore_virtual_mounts(session_id).await?;
        if req
            .path_pattern
            .as_deref()
            .is_some_and(|p| crate::domains::knowledge::runtime::is_mount(p.trim_end_matches('/')))
        {
            let matches = crate::domains::knowledge::KnowledgeService::files(self.db.clone())
                .mount_grep(session_id, &req.pattern, req.path_pattern.as_deref())
                .await?;
            return Ok(matches
                .into_iter()
                .map(|m| GrepResult {
                    path: m.path.clone(),
                    matches: vec![m],
                })
                .collect());
        }
        let mut results = grep_session_files_excluding(
            &self.db,
            session_id,
            &req.pattern,
            req.path_pattern.as_deref(),
            req.excluded_path_prefix.as_deref(),
        )
        .await?;

        // Also search virtual mounts (same per-file and NFA caps, TM-DOS-008)
        if let Some(registry) = &self.virtual_registry {
            let regex = build_grep_regex(&req.pattern)?;
            let path_matcher = req
                .path_pattern
                .as_deref()
                .map(everruns_core::session_path::GrepPathPattern::new)
                .transpose()?;
            // Apply the shared path matcher after reading virtual entries.
            let virtual_matches = registry.grep(
                &session_id,
                &regex,
                None,
                req.excluded_path_prefix.as_deref(),
                MAX_GREP_FILE_BYTES as usize,
            );
            for vm in virtual_matches.into_iter().filter(|vm| {
                path_matcher
                    .as_ref()
                    .is_none_or(|matcher| matcher.is_match(&vm.path))
            }) {
                // Group by file path into GrepResult entries
                if let Some(existing) = results.iter_mut().find(|r| r.path == vm.path) {
                    existing.matches.push(GrepMatch {
                        path: vm.path,
                        line_number: vm.line_number,
                        line: vm.line,
                    });
                } else {
                    results.push(GrepResult {
                        path: vm.path.clone(),
                        matches: vec![GrepMatch {
                            path: vm.path,
                            line_number: vm.line_number,
                            line: vm.line,
                        }],
                    });
                }
            }
        }

        // Mounted Memory content is part of the namespace the agent greps, and
        // `/memory/user` privacy is enforced by the caller's path-prefix
        // exclusion, which is applied to these paths too.
        {
            let router = &self.memory_mounts;
            let regex = build_grep_regex(&req.pattern)?;
            let path_matcher = req
                .path_pattern
                .as_deref()
                .map(everruns_core::session_path::GrepPathPattern::new)
                .transpose()?;
            for mount in router.all_mounts(session_id).await.iter() {
                if req
                    .excluded_path_prefix
                    .as_deref()
                    .is_some_and(|prefix| mount.mount_path.starts_with(prefix))
                {
                    continue;
                }
                for row in router.list_all(mount).await? {
                    if row.is_directory || row.size_bytes > MAX_GREP_FILE_BYTES {
                        continue;
                    }
                    let display = Self::memory_session_path(mount, &row.path);
                    if path_matcher
                        .as_ref()
                        .is_some_and(|matcher| !matcher.is_match(&display))
                    {
                        continue;
                    }
                    if req
                        .excluded_path_prefix
                        .as_deref()
                        .is_some_and(|prefix| display.starts_with(prefix))
                    {
                        continue;
                    }
                    let Some(bytes) = router.read_file(mount, &row.path).await? else {
                        continue;
                    };
                    let Ok(text) = String::from_utf8(bytes) else {
                        continue;
                    };
                    let matches: Vec<GrepMatch> = text
                        .lines()
                        .enumerate()
                        .filter(|(_, line)| regex.is_match(line))
                        .map(|(index, line)| GrepMatch {
                            path: display.clone(),
                            line_number: index + 1,
                            line: line.to_string(),
                        })
                        .collect();
                    if !matches.is_empty() {
                        results.push(GrepResult {
                            path: display,
                            matches,
                        });
                    }
                }
            }
        }

        Ok(results)
    }
}
