use super::RawMcpResponse;
use crate::EgressStreamResponse;
use anyhow::{Result, anyhow};
use futures::StreamExt;

const DEFAULT_MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAX_ERROR_BODY_BYTES: usize = 8 * 1024;

pub(super) fn configured_limit(value: Option<&str>) -> Result<usize> {
    match value {
        None => Ok(DEFAULT_MAX_RESPONSE_BYTES),
        Some(value) => value
            .parse::<usize>()
            .ok()
            .filter(|limit| *limit > 0)
            .ok_or_else(|| {
                anyhow!("EVERRUNS_MCP_MAX_RESPONSE_BYTES must be a positive byte count")
            }),
    }
}

/// THREAT[TM-DOS-054]: count bytes before retaining each chunk and drop the
/// stream at the bound. Content-Length is not trusted, including for errors.
pub(super) async fn read(
    response: EgressStreamResponse,
    limit: usize,
    discovery: bool,
    deadline: tokio::time::Instant,
) -> Result<RawMcpResponse> {
    let success = (200..300).contains(&response.status);
    let limit = if success { limit } else { MAX_ERROR_BODY_BYTES };
    let mut bytes = Vec::with_capacity(limit.min(16 * 1024));
    let mut stream = response.body;
    loop {
        let chunk = match tokio::time::timeout_at(deadline, stream.next()).await {
            Ok(Some(Ok(chunk))) => chunk,
            Ok(None) => break,
            // The status must survive body failures, especially 401/403 for
            // credential refresh and cache invalidation. Keep only the excerpt.
            Ok(Some(Err(_))) | Err(_) if !success => break,
            Ok(Some(Err(error))) => {
                return Err(anyhow!("Failed to read MCP response body: {error}"));
            }
            Err(_) => return Err(anyhow!("MCP response timed out")),
        };
        let remaining = limit - bytes.len();
        if !success {
            bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
            if bytes.len() == limit {
                break;
            }
        } else if chunk.len() > remaining {
            if discovery {
                return Err(crate::mcp::McpDiscoveryError::LimitExceeded {
                    resource: "page bytes",
                    limit,
                }
                .into());
            }
            return Err(anyhow!("MCP HTTP response exceeds {limit} bytes"));
        } else {
            bytes.extend_from_slice(&chunk);
        }
    }
    // Decode once so UTF-8 split across chunks remains intact. Error excerpts
    // can end inside a character or contain invalid bytes; never lose status.
    let body = if success {
        String::from_utf8(bytes).map_err(|e| anyhow!("Failed to read MCP response body: {e}"))?
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    Ok(RawMcpResponse {
        status: response.status,
        headers: response.headers,
        body,
    })
}
