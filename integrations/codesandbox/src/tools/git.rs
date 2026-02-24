//! Git clone tool for cloning repositories into sandboxes.

use crate::client::CodeSandboxClient;
use crate::state::*;
use crate::tools::exec::poll_exec_completion;

use async_trait::async_trait;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;
use serde_json::{Value, json};
use tracing::{debug, warn};

// ----------------------------------------------------------------------------
// CsbGitCloneTool
// ----------------------------------------------------------------------------

pub struct CsbGitCloneTool;

#[async_trait]
impl Tool for CsbGitCloneTool {
    fn name(&self) -> &str {
        "csb_git_clone"
    }

    fn description(&self) -> &str {
        "Clone a git repository into a CodeSandbox VM. Automatically uses the user's \
         connected GitHub credentials (GITHUB_TOKEN) if available. For private repos, \
         the user must have connected their GitHub account in Settings > Connections."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {
                    "type": "string",
                    "description": "Sandbox ID to clone into"
                },
                "repo_url": {
                    "type": "string",
                    "description": "Repository URL (e.g., 'https://github.com/user/repo' or 'user/repo' shorthand)"
                },
                "branch": {
                    "type": "string",
                    "description": "Branch to clone (optional, defaults to default branch)"
                },
                "path": {
                    "type": "string",
                    "description": "Clone destination path inside sandbox (optional, defaults to /sandbox/<owner>/<repo>)"
                }
            },
            "required": ["sandbox_id", "repo_url"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "csb_git_clone requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = match required_str(&arguments, "sandbox_id") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let repo_url_raw = match required_str(&arguments, "repo_url") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let branch = arguments.get("branch").and_then(|v| v.as_str());
        let clone_path = arguments.get("path").and_then(|v| v.as_str());

        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(s) => s,
            Err(e) => return e,
        };

        // Normalize repo URL: "user/repo" → "https://github.com/user/repo.git"
        let repo_url = normalize_repo_url(repo_url_raw);

        // Build default clone path: /sandbox/owner/repo (preserves user/repo structure)
        let default_path = if let Some(owner_repo) = extract_owner_repo(repo_url_raw) {
            format!("{}/{}", state.workspace_path, owner_repo)
        } else {
            let repo_name = repo_url
                .rsplit('/')
                .next()
                .unwrap_or("repo")
                .trim_end_matches(".git");
            format!("{}/{}", state.workspace_path, repo_name)
        };
        let target_path = clone_path.unwrap_or(&default_path);

        // Try to get GITHUB_TOKEN from session secrets for authentication
        let github_token = get_github_token(context).await;

        let client = CodeSandboxClient::new(api_key);

        // Step 1: Set up git credential helper if we have a token
        if let Some(ref token) = github_token {
            debug!("Setting up git credential helper for authenticated clone");
            let credential_script = format!(
                r#"#!/bin/sh
echo "protocol=https"
echo "host=github.com"
echo "username=oauth2"
echo "password={token}""#
            );

            // Write credential helper script
            let write_helper = client
                .exec_create(
                    &state,
                    "bash",
                    vec![
                        "-c".to_string(),
                        format!(
                            "mkdir -p /tmp && cat > /tmp/git-credential-helper.sh << 'CREDEOF'\n{credential_script}\nCREDEOF\nchmod +x /tmp/git-credential-helper.sh"
                        ),
                    ],
                )
                .await;

            if let Err(e) = write_helper {
                warn!("Failed to write credential helper: {e}");
                // Continue without auth — will fail for private repos
            } else if let Ok(exec_info) = write_helper {
                let _ = poll_exec_completion(&client, &state, &exec_info.id).await;
            }

            // Configure git to use the credential helper
            let configure_git = client
                .exec_create(
                    &state,
                    "bash",
                    vec![
                        "-c".to_string(),
                        "git config --global credential.helper '/tmp/git-credential-helper.sh'"
                            .to_string(),
                    ],
                )
                .await;

            if let Ok(exec_info) = configure_git {
                let _ = poll_exec_completion(&client, &state, &exec_info.id).await;
            }
        }

        // Step 2: Build and run git clone command
        let mut clone_cmd = "git clone --depth 1".to_string();
        if let Some(b) = branch {
            clone_cmd.push_str(&format!(" --branch {b}"));
        }
        clone_cmd.push_str(&format!(" {repo_url} {target_path}"));

        debug!("Cloning repository: {clone_cmd}");
        let exec_info = match client
            .exec_create(&state, "bash", vec!["-c".to_string(), clone_cmd])
            .await
        {
            Ok(info) => info,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        // Poll for completion
        let output = match poll_exec_completion(&client, &state, &exec_info.id).await {
            Ok(o) => o,
            Err(e) => return e,
        };

        // Step 3: Get the HEAD commit SHA
        let commit_sha = {
            let sha_exec = client
                .exec_create(
                    &state,
                    "bash",
                    vec![
                        "-c".to_string(),
                        format!("cd {target_path} && git rev-parse --short HEAD"),
                    ],
                )
                .await;
            if let Ok(exec_info) = sha_exec {
                if let Ok(sha_output) = poll_exec_completion(&client, &state, &exec_info.id).await {
                    sha_output.trim().to_string()
                } else {
                    "unknown".to_string()
                }
            } else {
                "unknown".to_string()
            }
        };

        // Step 4: Clean up credential helper (security)
        if github_token.is_some() {
            let cleanup = client
                .exec_create(
                    &state,
                    "bash",
                    vec![
                        "-c".to_string(),
                        "rm -f /tmp/git-credential-helper.sh && git config --global --unset credential.helper"
                            .to_string(),
                    ],
                )
                .await;
            if let Ok(exec_info) = cleanup {
                let _ = poll_exec_completion(&client, &state, &exec_info.id).await;
            }
        }

        // Check if clone succeeded (non-empty output usually means error)
        if output.contains("fatal:") || output.contains("error:") {
            let error_lines: String = output.lines().take(5).collect::<Vec<_>>().join("\n");
            let hint = if github_token.is_none()
                && (output.contains("Authentication failed")
                    || output.contains("could not read Username")
                    || output.contains("Repository not found"))
            {
                "\n\nThis may be a private repository. The user can connect their GitHub account in Settings > Connections to enable authenticated cloning."
            } else {
                ""
            };
            return ToolExecutionResult::tool_error(format!(
                "Git clone failed: {error_lines}{hint}"
            ));
        }

        ToolExecutionResult::success(json!({
            "sandbox_id": sandbox_id,
            "repo_url": repo_url,
            "path": target_path,
            "branch": branch.unwrap_or("default"),
            "commit": commit_sha,
            "authenticated": github_token.is_some()
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

/// Normalize repository URL: "user/repo" → "https://github.com/user/repo.git"
pub fn normalize_repo_url(url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("git@") {
        url.to_string()
    } else if url.contains('/') && !url.contains(' ') {
        // Looks like "user/repo" shorthand
        format!("https://github.com/{url}.git")
    } else {
        url.to_string()
    }
}

/// Extract "owner/repo" from a git URL. Returns `Some("owner/repo")` for:
/// - `"owner/repo"` shorthand
/// - `"https://github.com/owner/repo"` or `"https://github.com/owner/repo.git"`
/// - `"git@github.com:owner/repo.git"`
///
/// Returns `None` if the URL doesn't match any recognized pattern.
fn extract_owner_repo(url: &str) -> Option<String> {
    // Shorthand: "owner/repo" (no protocol, no spaces, exactly one slash)
    if !url.contains("://") && !url.starts_with("git@") && url.contains('/') && !url.contains(' ') {
        let trimmed = url.trim_end_matches(".git");
        let parts: Vec<&str> = trimmed.splitn(3, '/').collect();
        if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            return Some(trimmed.to_string());
        }
    }

    // HTTPS: "https://github.com/owner/repo" or "https://github.com/owner/repo.git"
    if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    {
        // Skip the host (e.g. "github.com/owner/repo.git" → "owner/repo.git")
        if let Some(path) = rest.split_once('/').map(|(_, p)| p) {
            let path = path.trim_end_matches(".git").trim_end_matches('/');
            let parts: Vec<&str> = path.splitn(3, '/').collect();
            if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
                return Some(path.to_string());
            }
        }
    }

    // SSH: "git@github.com:owner/repo.git"
    if let Some(rest) = url.strip_prefix("git@")
        && let Some(path) = rest.split_once(':').map(|(_, p)| p)
    {
        let path = path.trim_end_matches(".git").trim_end_matches('/');
        let parts: Vec<&str> = path.splitn(3, '/').collect();
        if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            return Some(path.to_string());
        }
    }

    None
}

/// Resolve GitHub token lazily from user connections, with session secret fallback.
async fn get_github_token(context: &ToolContext) -> Option<String> {
    // Try lazy resolution from user connections (preferred: always fresh)
    if let Some(ref resolver) = context.connection_resolver {
        match resolver
            .get_connection_token(context.session_id, "github")
            .await
        {
            Ok(Some(token)) if !token.is_empty() => return Some(token),
            Ok(_) => {}
            Err(e) => debug!("Connection resolver failed: {e}"),
        }
    }

    // Fallback: session secret (for backward compat with pre-injected tokens)
    if let Some(ref storage) = context.storage_store {
        match storage.get_secret(context.session_id, "GITHUB_TOKEN").await {
            Ok(Some(token)) if !token.is_empty() => return Some(token),
            Ok(_) => {}
            Err(e) => debug!("No GITHUB_TOKEN session secret: {e}"),
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::tools::Tool;

    #[test]
    fn test_normalize_repo_url_shorthand() {
        assert_eq!(
            normalize_repo_url("user/repo"),
            "https://github.com/user/repo.git"
        );
    }

    #[test]
    fn test_normalize_repo_url_https() {
        let url = "https://github.com/user/repo";
        assert_eq!(normalize_repo_url(url), url);
    }

    #[test]
    fn test_normalize_repo_url_http() {
        let url = "http://github.com/user/repo";
        assert_eq!(normalize_repo_url(url), url);
    }

    #[test]
    fn test_normalize_repo_url_git_ssh() {
        let url = "git@github.com:user/repo.git";
        assert_eq!(normalize_repo_url(url), url);
    }

    #[test]
    fn test_normalize_repo_url_plain_string() {
        // No slash, no known prefix → pass through unchanged
        assert_eq!(normalize_repo_url("myrepo"), "myrepo");
    }

    #[test]
    fn test_normalize_repo_url_with_spaces() {
        // Spaces prevent shorthand detection
        assert_eq!(normalize_repo_url("user /repo"), "user /repo");
    }

    #[test]
    fn test_normalize_repo_url_org_repo() {
        assert_eq!(
            normalize_repo_url("org/sub-repo"),
            "https://github.com/org/sub-repo.git"
        );
    }

    // --- extract_owner_repo tests ---

    #[test]
    fn test_extract_owner_repo_shorthand() {
        assert_eq!(
            extract_owner_repo("user/repo"),
            Some("user/repo".to_string())
        );
    }

    #[test]
    fn test_extract_owner_repo_https() {
        assert_eq!(
            extract_owner_repo("https://github.com/user/repo"),
            Some("user/repo".to_string())
        );
    }

    #[test]
    fn test_extract_owner_repo_https_git_suffix() {
        assert_eq!(
            extract_owner_repo("https://github.com/user/repo.git"),
            Some("user/repo".to_string())
        );
    }

    #[test]
    fn test_extract_owner_repo_ssh() {
        assert_eq!(
            extract_owner_repo("git@github.com:user/repo.git"),
            Some("user/repo".to_string())
        );
    }

    #[test]
    fn test_extract_owner_repo_bare_word() {
        assert_eq!(extract_owner_repo("something"), None);
    }

    #[test]
    fn test_extract_owner_repo_with_spaces() {
        assert_eq!(extract_owner_repo("user /repo"), None);
    }

    #[test]
    fn test_git_clone_schema() {
        let tool = CsbGitCloneTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(required_strs.contains(&"sandbox_id"));
        assert!(required_strs.contains(&"repo_url"));
        assert!(!required_strs.contains(&"branch"));
        assert!(!required_strs.contains(&"path"));
    }

    #[tokio::test]
    async fn test_git_clone_without_context() {
        let tool = CsbGitCloneTool;
        let result = tool
            .execute(json!({"sandbox_id": "test", "repo_url": "user/repo"}))
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("requires context")),
            _ => panic!("Expected tool error"),
        }
    }

    #[test]
    fn test_git_clone_requires_context() {
        assert!(CsbGitCloneTool.requires_context());
    }
}
