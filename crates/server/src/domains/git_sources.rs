use git2::{Error, ErrorCode};

pub const INVALID_GITHUB_REPOSITORY: &str = "GitHub repository must be owner/repo or https://github.com/owner/repo (optionally ending in .git)";

/// Normalize user-provided and historically stored GitHub coordinates to the
/// credential-free `owner/repo` form persisted by the API.
pub fn normalize_github_repository(input: &str) -> Result<String, &'static str> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.len() > 2048 || trimmed.contains(char::is_whitespace) {
        return Err(INVALID_GITHUB_REPOSITORY);
    }

    let path = if trimmed.contains("://") {
        let parsed = url::Url::parse(trimmed).map_err(|_| INVALID_GITHUB_REPOSITORY)?;
        if parsed.scheme() != "https"
            || parsed.host_str() != Some("github.com")
            || parsed.port().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(INVALID_GITHUB_REPOSITORY);
        }
        parsed.path().to_string()
    } else if let Some(path) = trimmed.strip_prefix("git@github.com:") {
        path.to_string()
    } else {
        trimmed.to_string()
    };

    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() != 2 || parts.iter().any(|part| !valid_github_component(part)) {
        return Err(INVALID_GITHUB_REPOSITORY);
    }

    Ok(format!("{}/{}", parts[0], parts[1]))
}

pub fn github_clone_url(repository: &str) -> Result<(String, String), &'static str> {
    let repository = normalize_github_repository(repository)?;
    Ok((format!("https://github.com/{repository}.git"), repository))
}

/// Convert libgit2 failures into credential-safe guidance stored for users.
pub fn safe_git_clone_error(
    error: &Error,
    github_source: bool,
    authenticated: bool,
) -> &'static str {
    let message = error.message();
    if error.code() == ErrorCode::Auth || message.contains("401") || message.contains("403") {
        return if github_source {
            "Repository authentication failed. Reconnect GitHub and retry"
        } else {
            "Repository authentication failed. Check repository access and retry"
        };
    }
    if error.code() == ErrorCode::NotFound || message.contains("404") {
        return if !github_source {
            "Repository or branch was not found. Check the repository URL and branch"
        } else if authenticated {
            "Repository or branch was not found, or access was denied. Check the repository and GitHub connection"
        } else {
            "Repository or branch was not found, or the repository is private. Check the repository or connect GitHub for private access"
        };
    }
    "Could not reach the repository. Check the network and retry"
}

fn valid_github_component(part: &str) -> bool {
    !part.is_empty()
        && part.len() <= 100
        && part
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use git2::ErrorClass;

    #[test]
    fn normalizes_common_github_inputs() {
        for input in [
            "everruns/bashkit",
            " everruns/bashkit.git ",
            "https://github.com/everruns/bashkit",
            "https://github.com/everruns/bashkit/",
            "https://github.com/everruns/bashkit.git",
            "https://github.com/everruns/bashkit.git/",
            "git@github.com:everruns/bashkit.git",
        ] {
            assert_eq!(
                normalize_github_repository(input),
                Ok("everruns/bashkit".to_string()),
                "input: {input}"
            );
        }
    }

    #[test]
    fn rejects_unsupported_or_malformed_github_inputs() {
        for input in [
            "",
            "bashkit",
            "owner/repo/extra",
            "http://github.com/owner/repo",
            "https://gitlab.com/owner/repo",
            "https://user@github.com/owner/repo",
            "https://github.com/owner/repo?token=secret",
            "https://github.com/owner/repo#readme",
        ] {
            assert_eq!(
                normalize_github_repository(input),
                Err(INVALID_GITHUB_REPOSITORY),
                "input: {input}"
            );
        }
    }

    #[test]
    fn clone_errors_are_actionable_without_provider_details() {
        let not_found = Error::new(ErrorCode::NotFound, ErrorClass::Net, "HTTP 404");
        assert!(safe_git_clone_error(&not_found, true, false).contains("connect GitHub"));
        let auth = Error::new(ErrorCode::Auth, ErrorClass::Net, "HTTP 401");
        assert_eq!(
            safe_git_clone_error(&auth, true, true),
            "Repository authentication failed. Reconnect GitHub and retry"
        );
        let network = Error::new(ErrorCode::Timeout, ErrorClass::Net, "internal hostname");
        assert_eq!(
            safe_git_clone_error(&network, true, false),
            "Could not reach the repository. Check the network and retry"
        );
        assert_eq!(
            safe_git_clone_error(&not_found, false, false),
            "Repository or branch was not found. Check the repository URL and branch"
        );
    }
}
