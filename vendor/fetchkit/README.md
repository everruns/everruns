# fetchkit

AI-friendly web content fetching tool designed for LLM consumption. Rust library with CLI, MCP server, and Python bindings.

## Features

- **HTTP fetching** - GET and HEAD methods with streaming support
- **Pluggable fetchers** - URL-aware dispatch to specialized handlers for repos, docs, feeds, videos, papers, and more
- **HTML-to-Markdown** - Built-in conversion optimized for LLMs
- **HTML-to-Text** - Plain text extraction with clean formatting
- **Binary detection** - Returns metadata only for images, PDFs, etc.
- **Timeout handling** - 1s first-byte, 30s body with partial content on timeout
- **Safety limits** - 10 MB default decompressed body cap with truncation
- **URL filtering** - URL-aware allow/block lists for controlled access
- **SSRF protection** - Resolve-then-check blocks private IPs by default
- **MCP server** - Model Context Protocol support for AI tool integration

## Built-in Fetchers

FetchKit routes each request through an ordered fetcher registry. Specialized
fetchers match first; the default fetcher handles everything else.

- `GitHubCodeFetcher` - GitHub source file URLs (`/blob/...`)
- `GitHubIssueFetcher` - GitHub issue and pull request URLs
- `GitHubRepoFetcher` - GitHub repository home pages
- `TwitterFetcher` - X/Twitter status URLs
- `StackOverflowFetcher` - Stack Overflow and Stack Exchange question URLs
- `PackageRegistryFetcher` - PyPI, crates.io, and npm package pages
- `WikipediaFetcher` - Wikipedia article URLs
- `YouTubeFetcher` - YouTube watch and `youtu.be` URLs
- `ArXivFetcher` - arXiv abstract and PDF URLs
- `HackerNewsFetcher` - Hacker News item threads
- `RSSFeedFetcher` - RSS and Atom feed URLs
- `DocsSiteFetcher` - docs sites with `llms.txt`/`llms-full.txt` support
- `DefaultFetcher` - all remaining HTTP/HTTPS URLs with HTML conversion, streaming, timeout handling, and binary detection

## Installation

### From crates.io (recommended)

```bash
cargo install fetchkit-cli
```

### From Git

```bash
cargo install --git https://github.com/everruns/fetchkit fetchkit-cli
```

### From Source

```bash
git clone https://github.com/everruns/fetchkit
cd fetchkit
cargo install --path crates/fetchkit-cli
```

## CLI Usage

```bash
# Fetch URL (outputs markdown with frontmatter)
fetchkit fetch https://example.com

# Output as JSON instead
fetchkit fetch https://example.com -o json

# Custom user agent
fetchkit fetch https://example.com --user-agent "MyBot/1.0"

# Hardened outbound policy for cluster/data-plane use
fetchkit fetch https://example.com --hardened

# Show full documentation
fetchkit --llmtxt
```

Default output is markdown with YAML frontmatter:

```markdown
---
url: https://example.com
status_code: 200
source_content_type: text/html; charset=UTF-8
source_size: 1256
---
# Example Domain

This domain is for use in illustrative examples in documents...
```

JSON output (`-o json`):

```json
{
  "url": "https://example.com",
  "status_code": 200,
  "content_type": "text/html",
  "size": 1256,
  "format": "markdown",
  "content": "# Example Domain\n\nThis domain is for use in illustrative examples..."
}
```

## MCP Server

Run as a Model Context Protocol server:

```bash
fetchkit mcp

# Hardened profile for cluster/data-plane use
fetchkit mcp --hardened
```

Exposes `fetchkit` tool over JSON-RPC 2.0 stdio transport. Returns markdown with frontmatter (same format as CLI). Compatible with Claude Desktop and other MCP clients.

## Library Usage

Add to `Cargo.toml`:

```toml
[dependencies]
fetchkit = "0.2"
```

### Basic Fetch

```rust
use fetchkit::{fetch, FetchRequest};

#[tokio::main]
async fn main() {
    let request = FetchRequest::new("https://example.com").as_markdown();

    let response = fetch(request).await.unwrap();
    println!("{}", response.content.unwrap_or_default());
}
```

### With Tool Builder

```rust
use fetchkit::{FetchRequest, ToolBuilder};

let tool = ToolBuilder::new()
    .enable_markdown(true)
    .enable_text(false)
    .user_agent("MyBot/1.0")
    .allow_prefix("https://docs.example.com")
    .block_prefix("https://internal.example.com")
    .build();

let request = FetchRequest::new("https://example.com");
let response = tool.execute(request).await.unwrap();
```

### Toolkit Contract Surface

```rust
use fetchkit::ToolBuilder;

let builder = ToolBuilder::new().enable_save_to_file(true);
let tool = builder.build();

assert_eq!(tool.name(), "web_fetch");
assert_eq!(tool.display_name(), "Web Fetch");

let definition = builder.build_tool_definition();
let mut service = builder.build_service();
```

### Hardened Tool Profile

```rust
use fetchkit::Tool;

let tool = Tool::builder()
    .hardened()
    .allow_prefix("https://docs.example.com")
    .build();
```

## Python Bindings

```bash
pip install fetchkit
```

```python
from fetchkit_py import fetch, FetchRequest, FetchKitTool

# Simple fetch
response = fetch("https://example.com", as_markdown=True)
print(response.content)

# With configuration
tool = FetchKitTool(
    enable_markdown=True,
    user_agent="MyBot/1.0",
    allow_prefixes=["https://docs.example.com"]
)
response = tool.fetch("https://example.com")
```

## Request Fields

| Field | Type | Description |
|-------|------|-------------|
| `url` | string | URL to fetch (required, `http://` or `https://`) |
| `method` | enum? | `GET` (default) or `HEAD` |
| `as_markdown` | bool? | Convert HTML to markdown |
| `as_text` | bool? | Convert HTML to plain text |
| `save_to_file` | string? | Save body to path (requires `FileSaver`) |
| `content_focus` | string? | `"main"` strips boilerplate; `"full"`/unset returns everything |
| `if_none_match` | string? | ETag for conditional `If-None-Match` |
| `if_modified_since` | string? | Timestamp for conditional `If-Modified-Since` |

## Response Fields

| Field | Type | Description |
|-------|------|-------------|
| `url` | string | Fetched URL |
| `status_code` | int | HTTP status code |
| `content_type` | string? | Content-Type header |
| `size` | int? | Content size in bytes |
| `last_modified` | string? | Last-Modified header |
| `etag` | string? | ETag header (use for next conditional request) |
| `filename` | string? | From Content-Disposition |
| `format` | string? | `"markdown"`, `"text"`, `"raw"`, or a fetcher-specific format |
| `content` | string? | Page content |
| `truncated` | bool? | True if content was cut off |
| `method` | string? | `"HEAD"` for HEAD requests |
| `error` | string? | Error message if failed |
| `saved_path` | string? | Filesystem path when `save_to_file` succeeded |
| `bytes_written` | int? | Bytes saved to file |
| `metadata` | object? | Structured `PageMetadata` (title, description, links, headings, …) |
| `word_count` | int? | Word count of returned content |
| `redirect_chain` | string[] | URLs visited during redirects (empty if none) |
| `is_paywall` | bool? | Heuristic paywall signal (soft, not guaranteed) |

## Error Handling

Errors are returned in the `error` field:

- `InvalidUrl` - Malformed URL
- `UrlBlocked` - URL blocked by filter
- `NetworkError` - Connection failed
- `Timeout` - Request timed out
- `HttpError` - 4xx/5xx response
- `ContentError` - Failed to read body
- `BinaryContent` - Binary content not supported

## Security

FetchKit blocks connections to private/reserved IP ranges by default, preventing SSRF attacks when used in server-side or AI agent contexts.

**Blocked by default:** loopback, private networks (10.x, 172.16-31.x, 192.168.x), link-local (169.254.x including cloud metadata), IPv6 equivalents, multicast, and other reserved ranges.

```rust
// Default: private IPs blocked (safe for production)
let tool = Tool::default();

// Explicit opt-out for local development only
let tool = Tool::builder()
    .block_private_ips(false)
    .build();
```

DNS pinning prevents DNS rebinding attacks. IPv6-mapped IPv4 addresses are canonicalized before validation.
Redirects are followed manually in the default fetcher so each hop is revalidated against scheme and DNS policy. Allow/block prefixes are matched against parsed URLs rather than raw strings, which prevents lookalike host overmatches such as `allowed.example.com.evil.test`.
Proxy environment variables are ignored by default. Use the hardened profile for cluster-facing deployments and opt in with `ToolBuilder::respect_proxy_env(true)` only when it is part of an intentional egress design.

See [`specs/threat-model.md`](specs/threat-model.md) for the full threat model.
See [`docs/hardening.md`](docs/hardening.md) for deployment guidance.

## Configuration

### Timeouts And Limits

- **First-byte**: 1 second (connect + initial response)
- **Body**: 30 seconds total
- **Body size**: 10 MB decompressed content by default

Partial content is returned on body timeout or body-size limit with `truncated: true`.

### Binary Content

Automatically detected and returns metadata only for:
- Images, audio, video, fonts
- PDFs, archives (zip, tar, rar, 7z)
- Office documents

### HTML Conversion

HTML is automatically converted to markdown:
- Headers: `h1-h6` → `#` to `######`
- Lists: Proper nesting with 2-space indent
- Code: Fenced blocks and inline backticks
- Links: `[text](url)` format
- Strips: scripts, styles, iframes, SVGs

## License

MIT
