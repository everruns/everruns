//! Application-facing MCP server configuration.

use std::{collections::HashSet, fmt};

use everruns_core::{McpServerTransportType, ScopedMcpServer, sanitize_mcp_server_name};

/// A scoped MCP server available to an agent.
///
/// Use [`McpServer::stdio`] for a local child process (requires the
/// `mcp-stdio` feature), or [`McpServer::http`] for a remote Streamable HTTP
/// endpoint. Header and environment values are redacted from `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct McpServer {
    pub(crate) name: String,
    pub(crate) inner: ScopedMcpServer,
}

impl McpServer {
    /// Configure a remote Streamable HTTP MCP server.
    pub fn http(name: impl Into<String>, url: impl Into<String>) -> Self {
        // THREAT[TM-TOOL-018]: request-time MCP transport performs DNS-pinned
        // SSRF validation; this value-only builder does not create a bypass.
        Self {
            name: name.into(),
            inner: ScopedMcpServer {
                transport_type: McpServerTransportType::Http,
                url: url.into(),
                ..ScopedMcpServer::default()
            },
        }
    }

    /// Configure a local MCP server launched as a child process over stdio.
    ///
    /// The command, arguments, and environment must come from trusted host
    /// configuration, never model output or untrusted request input.
    #[cfg(feature = "mcp-stdio")]
    pub fn stdio(name: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            inner: ScopedMcpServer {
                transport_type: McpServerTransportType::Stdio,
                command: Some(command.into()),
                ..ScopedMcpServer::default()
            },
        }
    }

    /// Append one command-line argument for a stdio server.
    #[cfg(feature = "mcp-stdio")]
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.inner.args.push(arg.into());
        self
    }

    /// Append command-line arguments for a stdio server.
    #[cfg(feature = "mcp-stdio")]
    pub fn args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.inner.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Set one child-process environment variable for a stdio server.
    #[cfg(feature = "mcp-stdio")]
    pub fn env(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.inner.env.insert(name.into(), value.into());
        self
    }

    /// Set one literal HTTP header for a remote server.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.inner.headers.insert(name.into(), value.into());
        self
    }

    /// Disable live tool discovery for this server.
    pub fn tool_discovery(mut self, enabled: bool) -> Self {
        self.inner.tool_discovery = enabled;
        self
    }

    /// Server name used as the MCP tool namespace.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Debug for McpServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // THREAT[TM-TOOL-007]: header/environment values and executable
        // details may carry credentials, so Debug reports shape only.
        let header_names = self.inner.headers.keys().cloned().collect::<Vec<_>>();
        let env_names = self.inner.env.keys().cloned().collect::<Vec<_>>();
        f.debug_struct("McpServer")
            .field("name", &self.name)
            .field("transport", &self.inner.transport_type)
            .field("url_configured", &!self.inner.url.is_empty())
            .field("command_configured", &self.inner.command.is_some())
            .field("arg_count", &self.inner.args.len())
            .field("header_names", &header_names)
            .field("env_names", &env_names)
            .field("tool_discovery", &self.inner.tool_discovery)
            .finish()
    }
}

pub(crate) fn into_scoped(
    servers: Vec<McpServer>,
) -> Result<everruns_core::ScopedMcpServers, String> {
    let mut scoped = everruns_core::ScopedMcpServers::new();
    let mut sanitized_names = HashSet::new();
    for server in servers {
        let name = server.name.trim();
        if name.is_empty() {
            return Err("MCP server name must not be blank".to_string());
        }
        let sanitized_name = sanitize_mcp_server_name(name);
        if sanitized_name.contains("__") {
            return Err(format!(
                "MCP server name {name:?} is invalid after sanitization: consecutive underscores are reserved"
            ));
        }
        if !sanitized_names.insert(sanitized_name) {
            return Err("MCP server names must be unique after sanitization".to_string());
        }
        match server.inner.transport_type {
            McpServerTransportType::Http if server.inner.url.trim().is_empty() => {
                return Err(format!("HTTP MCP server {name:?} requires a URL"));
            }
            McpServerTransportType::Stdio
                if server
                    .inner
                    .command
                    .as_deref()
                    .is_none_or(|command| command.trim().is_empty()) =>
            {
                return Err(format!("stdio MCP server {name:?} requires a command"));
            }
            _ => {}
        }
        if scoped.insert(name.to_string(), server.inner).is_some() {
            return Err(format!("duplicate MCP server {name:?}"));
        }
    }
    Ok(scoped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_http_credentials_and_endpoint() {
        let server = McpServer::http("remote", "https://secret.example/token")
            .header("Authorization", "Bearer top-secret");

        let debug = format!("{server:?}");
        assert!(debug.contains("Authorization"));
        assert!(!debug.contains("top-secret"));
        assert!(!debug.contains("secret.example"));
    }

    #[cfg(feature = "mcp-stdio")]
    #[test]
    fn debug_redacts_stdio_command_arguments_and_environment_values() {
        let server = McpServer::stdio("local", "/private/bin/server")
            .arg("--api-key=top-secret")
            .env("API_KEY", "top-secret");

        let debug = format!("{server:?}");
        assert!(debug.contains("API_KEY"));
        assert!(!debug.contains("top-secret"));
        assert!(!debug.contains("/private/bin/server"));
    }
}
