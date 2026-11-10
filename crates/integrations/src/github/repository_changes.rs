//! What a session changed, per repository, read from its sandbox for people.
//!
//! Decisions:
//! - Same source as `repo_status`: the checkouts under the workspace root.
//!   Nothing here needs a GitHub token, so reading never mints one.
//! - Read-only. It informs a reviewer; pushing and discarding stay with the
//!   agent.
//! - Everything in a checkout is the agent's to write, so the output is
//!   parsed between markers the caller picks per request and is bounded: a
//!   repository cannot forge another repository's section or flood the reply.

use everruns_contracts::runtime::sandbox_workspace::SandboxWorkspace;
use everruns_contracts::runtime::tool_context::ToolContext;
use serde_json::{Value, json};

use super::repositories::{
    RepositoriesConfig, STATUS_TIMEOUT_MS, checkout_json, checkouts, sandbox,
};

/// Changed files reported per repository.
const MAX_FILES: usize = 200;
/// Bytes of unified diff kept per repository.
const MAX_DIFF_BYTES: usize = 12_000;

/// `MARK` is replaced with the caller's marker; `git` prints everything else.
const CHANGES_SCRIPT: &str = r#"for d in */; do
  d=${d%/}
  [ -d "$d/.git" ] || continue
  base=$(git -C "$d" config --get everruns.base 2>/dev/null || true)
  from=$(git -C "$d" merge-base "refs/remotes/origin/$base" HEAD 2>/dev/null || true)
  printf 'MARK repo\t%s\n' "$d"
  if [ -n "$from" ]; then
    git -C "$d" diff --numstat "$from" HEAD 2>/dev/null | head -n LIMIT | sed 's/^/MARK committed\t/'
  else
    from=HEAD
  fi
  git -C "$d" diff --numstat HEAD 2>/dev/null | head -n LIMIT | sed 's/^/MARK uncommitted\t/'
  git -C "$d" ls-files --others --exclude-standard 2>/dev/null | head -n LIMIT | sed 's/^/MARK untracked\t/'
  printf 'MARK diff\n'
  git -C "$d" diff "$from" 2>/dev/null | head -c BYTES
  printf '\nMARK end\n'
done
exit 0
"#;

#[derive(Debug, Default, PartialEq)]
struct RepositoryChanges {
    files: Vec<Value>,
    diff: String,
    diff_truncated: bool,
}

fn file(path: &str, additions: &str, deletions: &str, committed: bool) -> Value {
    json!({
        "path": path,
        // `-` marks a binary file.
        "additions": additions.parse::<u64>().ok(),
        "deletions": deletions.parse::<u64>().ok(),
        "committed": committed,
    })
}

/// Sections by checkout directory. Lines outside a section, or carrying
/// another marker, are ignored.
fn parse_changes(stdout: &str, marker: &str) -> Vec<(String, RepositoryChanges)> {
    let prefix = format!("{marker} ");
    let mut repositories: Vec<(String, RepositoryChanges)> = Vec::new();
    let mut in_diff = false;
    for line in stdout.lines() {
        let Some(rest) = line.strip_prefix(&prefix) else {
            if in_diff && let Some((_, current)) = repositories.last_mut() {
                current.diff.push_str(line);
                current.diff.push('\n');
            }
            continue;
        };
        in_diff = false;
        let fields: Vec<&str> = rest.split('\t').collect();
        match fields[..] {
            ["repo", directory] => {
                repositories.push((directory.to_string(), RepositoryChanges::default()))
            }
            ["diff"] => in_diff = true,
            ["end"] => {
                if let Some((_, current)) = repositories.last_mut() {
                    let diff = current.diff.trim_end_matches('\n').to_string();
                    current.diff_truncated = diff.len() >= MAX_DIFF_BYTES;
                    current.diff = diff;
                }
            }
            [
                kind @ ("committed" | "uncommitted"),
                additions,
                deletions,
                path,
            ] => {
                if let Some((_, current)) = repositories.last_mut()
                    && current.files.len() < MAX_FILES
                {
                    let committed = kind == "committed";
                    // A file changed again after its commit is listed once, as
                    // not committed.
                    current
                        .files
                        .retain(|existing| existing["path"] != json!(path));
                    current
                        .files
                        .push(file(path, additions, deletions, committed));
                }
            }
            ["untracked", path] => {
                if let Some((_, current)) = repositories.last_mut()
                    && current.files.len() < MAX_FILES
                {
                    current.files.push(json!({
                        "path": path,
                        "additions": null,
                        "deletions": null,
                        "committed": false,
                    }));
                }
            }
            _ => {}
        }
    }
    repositories
}

/// The session's checkouts with their changed files and a bounded diff, for
/// the `repositories` config in `config`. `marker` must be unguessable to the
/// sandbox (letters and digits only).
pub async fn read_repository_changes(
    context: &ToolContext,
    config: &Value,
    marker: &str,
) -> Result<Value, String> {
    if marker.is_empty() || !marker.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err("invalid marker".to_string());
    }
    let config = RepositoriesConfig::parse(config).unwrap_or_default();
    let sandbox = sandbox(context).map_err(|_| "This session has no sandbox".to_string())?;
    let (root, present) = checkouts(sandbox.as_ref(), context)
        .await
        .map_err(|_| "Could not read the repositories from the sandbox".to_string())?;
    let script = CHANGES_SCRIPT
        .replace("MARK", marker)
        .replace("LIMIT", &MAX_FILES.to_string())
        .replace("BYTES", &MAX_DIFF_BYTES.to_string());
    let changes = read(sandbox.as_ref(), context, &script, &root, marker).await;
    let work_branch = config.work_branch(context.session_id);
    let repositories: Vec<Value> = present
        .iter()
        .map(|checkout| {
            let mut entry = checkout_json(checkout, &config, &work_branch);
            let found = changes
                .iter()
                .find(|(directory, _)| directory == &checkout.directory);
            entry["files"] = json!(found.map(|(_, c)| c.files.clone()).unwrap_or_default());
            entry["diff"] = json!(found.map(|(_, c)| c.diff.clone()).unwrap_or_default());
            entry["diff_truncated"] = json!(found.is_some_and(|(_, c)| c.diff_truncated));
            entry
        })
        .collect();
    Ok(json!({ "work_branch": work_branch, "repositories": repositories }))
}

async fn read(
    sandbox: &dyn SandboxWorkspace,
    context: &ToolContext,
    script: &str,
    root: &str,
    marker: &str,
) -> Vec<(String, RepositoryChanges)> {
    match sandbox
        .exec_raw(context, script, Some(root), Some(STATUS_TIMEOUT_MS))
        .await
    {
        Ok(response) if response.exit_code == 0 => parse_changes(&response.stdout, marker),
        // The status above still stands; only the file lists are missing.
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTPUT: &str = "m1 repo\tlib\n\
        m1 committed\t6\t6\tsrc/config.rs\n\
        m1 committed\t1\t1\tsrc/lib.rs\n\
        m1 uncommitted\t2\t0\tsrc/lib.rs\n\
        m1 untracked\tnotes.txt\n\
        m1 committed\t-\t-\tlogo.png\n\
        m1 diff\n\
        diff --git a/src/config.rs b/src/config.rs\n\
        -old\n\
        +new\n\
        m2 repo\tforged\n\
        \n\
        m1 end\n\
        m1 repo\tdocs\n\
        m1 diff\n\
        \n\
        m1 end\n";

    #[test]
    fn files_and_diff_are_read_per_repository() {
        let parsed = parse_changes(OUTPUT, "m1");
        assert_eq!(parsed.len(), 2, "another marker opens no section");
        let (directory, lib) = &parsed[0];
        assert_eq!(directory, "lib");
        let paths: Vec<_> = lib
            .files
            .iter()
            .map(|f| {
                (
                    f["path"].as_str().unwrap(),
                    f["committed"].as_bool().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            paths,
            [
                ("src/config.rs", true),
                ("src/lib.rs", false),
                ("notes.txt", false),
                ("logo.png", true),
            ]
        );
        assert_eq!(lib.files[0]["additions"], 6);
        assert!(lib.files[3]["additions"].is_null(), "binary file");
        assert!(lib.diff.starts_with("diff --git a/src/config.rs"));
        assert!(lib.diff.contains("m2 repo\tforged"), "kept as diff text");
        assert!(!lib.diff_truncated);
        assert_eq!(
            parsed[1],
            ("docs".to_string(), RepositoryChanges::default())
        );
    }

    #[test]
    fn a_diff_at_the_limit_is_marked_truncated() {
        let output = format!("m repo\ta\nm diff\n{}\nm end\n", "x".repeat(MAX_DIFF_BYTES));
        assert!(parse_changes(&output, "m")[0].1.diff_truncated);
    }

    #[test]
    fn the_script_has_no_placeholder_left_and_ends_cleanly() {
        let script = CHANGES_SCRIPT
            .replace("MARK", "m")
            .replace("LIMIT", "200")
            .replace("BYTES", "12000");
        assert!(!script.contains("MARK") && !script.contains("LIMIT") && !script.contains("BYTES"));
        assert!(script.trim_end().ends_with("exit 0"));
    }
}
