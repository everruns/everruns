// Shallow git fetch over HTTPS, using the existing rustls HTTP client.
//
// Decision: libgit2 remains the object-store implementation (it backs
// session_git's Postgres-backed ODB), but its network transport is not used.
// Enabling git2's `https` feature links a second TLS stack into a workspace
// that is otherwise entirely rustls, and `vendored-openssl` compiles OpenSSL
// from source on every image build — which is also the only reason the builder
// stage installs perl and make. Speaking the smart-HTTP v2 protocol here keeps
// one TLS stack and drops that build.
//
// Scope is deliberately narrow: one shallow, single-branch, read-only checkout,
// which is all the knowledge-index and memory source syncs need. There is no
// negotiation, no push, and no incremental fetch. libgit2 still does the hard
// part — indexing the received packfile and materializing the tree.

use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};

/// Ceiling on a packfile response, overridable with `GIT_FETCH_MAX_PACK_BYTES`.
///
/// THREAT[TM-DOS-037]: the remote controls the response length, and a
/// source URL can point anywhere (memory sources accept arbitrary git URLs).
/// libgit2 used to stream the pack into its own object store; reading it here
/// means an unbounded body would be an unbounded allocation in the worker. The
/// default is generous against real repositories but far below the memory a
/// sync task may claim — and the snapshot the checkout feeds is itself capped
/// at 100 MB downstream, so a pack near this ceiling fails there anyway.
const DEFAULT_MAX_PACK_BYTES: u64 = 512 * 1024 * 1024;

/// Ceiling on the capability advertisement and ref listing, which are text and
/// orders of magnitude smaller than a pack.
const MAX_CONTROL_BYTES: u64 = 32 * 1024 * 1024;

fn max_pack_bytes() -> u64 {
    std::env::var("GIT_FETCH_MAX_PACK_BYTES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_MAX_PACK_BYTES)
}

/// How a fetch failed, at the granularity the user-facing guidance in
/// [`crate::domains::git_sources::safe_git_clone_error`] distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchFailure {
    /// The remote rejected our credentials.
    Auth,
    /// The repository or branch does not exist, or is invisible to us.
    NotFound,
    /// Anything else: DNS, TLS, timeouts, malformed protocol.
    Unreachable,
}

/// A fetch failure carrying both a classification for the user and the
/// underlying error for operator logs.
#[derive(Debug)]
pub struct FetchError {
    pub failure: FetchFailure,
    source: anyhow::Error,
}

impl FetchError {
    fn new(failure: FetchFailure, source: anyhow::Error) -> Self {
        Self { failure, source }
    }
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:#}", self.source)
    }
}

impl std::error::Error for FetchError {}

/// Maps an HTTP status onto a failure class.
fn classify_status(status: reqwest::StatusCode) -> FetchFailure {
    match status.as_u16() {
        401 | 403 => FetchFailure::Auth,
        404 | 410 => FetchFailure::NotFound,
        _ => FetchFailure::Unreachable,
    }
}

/// What to fetch, and how to authenticate.
pub struct FetchRequest<'a> {
    /// Repository URL, e.g. `https://github.com/owner/repo.git`.
    pub url: &'a str,
    /// Branch name without the `refs/heads/` prefix.
    pub branch: &'a str,
    /// Token sent as the HTTP basic password for user `x-access-token`.
    pub auth_token: Option<&'a str>,
}

/// A parsed pkt-line.
#[derive(Debug, PartialEq, Eq)]
enum PktLine<'a> {
    /// `0000`
    Flush,
    /// `0001`
    Delimiter,
    /// `0002`
    ResponseEnd,
    Data(&'a [u8]),
}

/// Encodes one pkt-line: a four-digit hex length covering the length prefix
/// itself, then the payload.
fn encode_pkt_line(payload: &str) -> Vec<u8> {
    let mut out = format!("{:04x}", payload.len() + 4).into_bytes();
    out.extend_from_slice(payload.as_bytes());
    out
}

/// Splits the next pkt-line off `input`, returning it and the remainder.
fn next_pkt_line(input: &[u8]) -> Result<(PktLine<'_>, &[u8])> {
    if input.len() < 4 {
        bail!("truncated pkt-line header");
    }
    let header = std::str::from_utf8(&input[..4]).context("pkt-line header is not ASCII")?;
    let length =
        usize::from_str_radix(header, 16).context("pkt-line header is not a hex length")?;

    match length {
        0 => Ok((PktLine::Flush, &input[4..])),
        1 => Ok((PktLine::Delimiter, &input[4..])),
        2 => Ok((PktLine::ResponseEnd, &input[4..])),
        // 3 is reserved and has no meaning; a length below the header size is
        // otherwise nonsense.
        3 => bail!("reserved pkt-line length 0003"),
        _ => {
            if length > input.len() {
                bail!(
                    "pkt-line claims {length} bytes but only {} remain",
                    input.len()
                );
            }
            Ok((PktLine::Data(&input[4..length]), &input[length..]))
        }
    }
}

/// Iterates every pkt-line in a complete response body.
fn parse_pkt_lines(mut input: &[u8]) -> Result<Vec<PktLine<'_>>> {
    let mut lines = Vec::new();
    while !input.is_empty() {
        let (line, rest) = next_pkt_line(input)?;
        lines.push(line);
        input = rest;
    }
    Ok(lines)
}

/// Builds the request body for the v2 `ls-refs` command, restricted to the one
/// branch being fetched.
fn ls_refs_body(branch: &str) -> Vec<u8> {
    let mut body = encode_pkt_line("command=ls-refs\n");
    body.extend_from_slice(b"0001");
    body.extend_from_slice(&encode_pkt_line(&format!(
        "ref-prefix refs/heads/{branch}\n"
    )));
    body.extend_from_slice(b"0000");
    body
}

/// Builds the request body for the v2 `fetch` command: one want, depth 1, and
/// an immediate `done` so the server does not wait for negotiation.
fn fetch_body(oid: &str) -> Vec<u8> {
    let mut body = encode_pkt_line("command=fetch\n");
    body.extend_from_slice(b"0001");
    for line in [
        "no-progress\n".to_string(),
        "deepen 1\n".to_string(),
        format!("want {oid}\n"),
        "done\n".to_string(),
    ] {
        body.extend_from_slice(&encode_pkt_line(&line));
    }
    body.extend_from_slice(b"0000");
    body
}

/// Finds the object id `ls-refs` reported for `refs/heads/<branch>`.
fn parse_ls_refs(body: &[u8], branch: &str) -> Result<String> {
    let wanted = format!("refs/heads/{branch}");
    for line in parse_pkt_lines(body)? {
        let PktLine::Data(data) = line else { continue };
        let text = String::from_utf8_lossy(data);
        let text = text.trim_end();
        let Some((oid, rest)) = text.split_once(' ') else {
            continue;
        };
        // Attributes such as `symref-target` follow the ref name, space separated.
        let name = rest.split(' ').next().unwrap_or(rest);
        if name == wanted {
            return Ok(oid.to_string());
        }
    }
    bail!("branch '{branch}' was not found on the remote")
}

/// Extracts packfile bytes from a v2 `fetch` response.
///
/// After the `packfile` section header every data line is sideband-multiplexed:
/// band 1 carries pack data, band 2 progress, band 3 a fatal error.
fn extract_packfile(body: &[u8]) -> Result<Vec<u8>> {
    let mut pack = Vec::new();
    let mut in_packfile_section = false;

    for line in parse_pkt_lines(body)? {
        let PktLine::Data(data) = line else {
            // A delimiter ends the current section; the next data line names
            // the section that follows.
            in_packfile_section = false;
            continue;
        };

        if !in_packfile_section {
            if data.starts_with(b"packfile") {
                in_packfile_section = true;
            }
            continue;
        }

        let Some((band, payload)) = data.split_first() else {
            continue;
        };
        match band {
            1 => pack.extend_from_slice(payload),
            2 => {} // progress; suppressed by `no-progress` but tolerated
            3 => bail!("remote error: {}", String::from_utf8_lossy(payload).trim()),
            other => bail!("unknown sideband {other} in packfile section"),
        }
    }

    if pack.is_empty() {
        bail!("remote returned no packfile");
    }
    Ok(pack)
}

/// Appends the smart-HTTP endpoint to a repository URL.
fn endpoint(url: &str, path: &str) -> String {
    format!("{}/{path}", url.trim_end_matches('/'))
}

/// Strips the URL from a reqwest error before it can reach a log.
///
/// THREAT[TM-API-018]: memory git sources accept an arbitrary URL, which may
/// embed credentials (`https://user:token@host/repo.git`). reqwest's error
/// Display includes the request URL, and `source_sync` logs these errors for
/// operators, so the URL is removed at the boundary rather than trusted to be
/// credential-free.
fn redact_url(error: reqwest::Error) -> reqwest::Error {
    error.without_url()
}

fn client_builder() -> reqwest::blocking::ClientBuilder {
    reqwest::blocking::Client::builder()
        // THREAT[TM-API-023]: source URLs are validated before storage, but a
        // remote-controlled redirect could otherwise bypass that validation
        // and reach an internal service. Git hosts do not require redirects
        // for their smart-HTTP endpoints, so fail closed on every 3xx.
        .redirect(reqwest::redirect::Policy::none())
}

fn client() -> Result<reqwest::blocking::Client> {
    client_builder()
        .build()
        .context("failed to build git HTTP client")
}

fn authenticated(
    builder: reqwest::blocking::RequestBuilder,
    auth_token: Option<&str>,
) -> reqwest::blocking::RequestBuilder {
    match auth_token {
        Some(token) => builder.basic_auth("x-access-token", Some(token)),
        None => builder,
    }
}

/// Fetches `branch` at depth 1 and materializes its tree into `checkout_dir`.
///
/// Blocking: libgit2 and the pack index are both synchronous, so callers run
/// this inside `spawn_blocking`, as they already did for `RepoBuilder::clone`.
pub fn shallow_checkout(
    request: &FetchRequest<'_>,
    checkout_dir: &Path,
) -> std::result::Result<(), FetchError> {
    let unreachable = |error: anyhow::Error| FetchError::new(FetchFailure::Unreachable, error);

    let client = client().map_err(unreachable)?;

    // Advertise v2 up front; a server that only speaks v1 will answer with a
    // v1 advertisement, which ensure_protocol_v2 rejects.
    let advertisement = authenticated(
        client
            .get(endpoint(request.url, "info/refs?service=git-upload-pack"))
            .header("Git-Protocol", "version=2"),
        request.auth_token,
    )
    .send()
    .map_err(redact_url)
    .context("failed to reach the git remote")
    .map_err(unreachable)?;

    let status = advertisement.status();
    if !status.is_success() {
        return Err(FetchError::new(
            classify_status(status),
            anyhow!("git remote returned status {status}"),
        ));
    }
    let advertisement = read_capped(
        advertisement,
        MAX_CONTROL_BYTES,
        "the git remote's capability advertisement",
    )
    .map_err(unreachable)?;
    ensure_protocol_v2(&advertisement).map_err(unreachable)?;

    let refs = post_upload_pack(
        &client,
        request,
        ls_refs_body(request.branch),
        "list remote refs",
        MAX_CONTROL_BYTES,
    )?;
    // A branch the remote does not advertise is indistinguishable, for the
    // user, from a repository they cannot see.
    let oid = parse_ls_refs(&refs, request.branch)
        .map_err(|error| FetchError::new(FetchFailure::NotFound, error))?;

    let response = post_upload_pack(
        &client,
        request,
        fetch_body(&oid),
        "fetch objects",
        max_pack_bytes(),
    )?;
    let packfile = extract_packfile(&response).map_err(unreachable)?;

    checkout_packfile(&packfile, &oid, checkout_dir).map_err(unreachable)
}

/// Posts one protocol-v2 command and returns the response body.
fn post_upload_pack(
    client: &reqwest::blocking::Client,
    request: &FetchRequest<'_>,
    body: Vec<u8>,
    what: &str,
    max_bytes: u64,
) -> std::result::Result<Vec<u8>, FetchError> {
    let response = authenticated(
        client
            .post(endpoint(request.url, "git-upload-pack"))
            .header("Content-Type", "application/x-git-upload-pack-request")
            .header("Git-Protocol", "version=2")
            .body(body),
        request.auth_token,
    )
    .send()
    .map_err(redact_url)
    .with_context(|| format!("failed to {what}"))
    .map_err(|error| FetchError::new(FetchFailure::Unreachable, error))?;

    let status = response.status();
    if !status.is_success() {
        return Err(FetchError::new(
            classify_status(status),
            anyhow!("git remote returned status {status} while trying to {what}"),
        ));
    }

    read_capped(response, max_bytes, what)
        .map_err(|error| FetchError::new(FetchFailure::Unreachable, error))
}

/// Reads a response body, refusing to allocate more than `max_bytes`.
///
/// Reads one byte past the ceiling so an over-long body is detected rather than
/// silently truncated into a malformed pkt-line stream. A declared
/// `Content-Length` over the ceiling is rejected before any body is read.
fn read_capped(
    response: reqwest::blocking::Response,
    max_bytes: u64,
    what: &str,
) -> Result<Vec<u8>> {
    if let Some(declared) = response.content_length()
        && declared > max_bytes
    {
        bail!("{what} is {declared} bytes, over the {max_bytes} byte limit");
    }

    let mut body = Vec::new();
    response
        .take(max_bytes + 1)
        .read_to_end(&mut body)
        .with_context(|| format!("failed to read {what}"))?;

    if body.len() as u64 > max_bytes {
        bail!("{what} exceeded the {max_bytes} byte limit");
    }
    Ok(body)
}

/// Rejects anything that is not a protocol v2 advertisement, so a v1 fallback
/// fails loudly instead of being misparsed.
fn ensure_protocol_v2(advertisement: &[u8]) -> Result<()> {
    for line in parse_pkt_lines(advertisement)? {
        if let PktLine::Data(data) = line
            && data.starts_with(b"version 2")
        {
            return Ok(());
        }
    }
    bail!("git remote did not offer protocol v2")
}

/// Indexes `packfile` into a repository at `checkout_dir` and checks out the
/// tree of `oid`.
fn checkout_packfile(packfile: &[u8], oid: &str, checkout_dir: &Path) -> Result<()> {
    let repository = git2::Repository::init(checkout_dir)
        .context("failed to initialize the checkout repository")?;

    {
        let odb = repository
            .odb()
            .context("failed to open the object store")?;
        let mut writer = odb
            .packwriter()
            .context("failed to open a packfile writer")?;
        writer
            .write_all(packfile)
            .context("failed to write the packfile")?;
        writer.commit().context("failed to index the packfile")?;
    }

    let oid = git2::Oid::from_str(oid).context("remote returned a malformed object id")?;
    let commit = repository
        .find_commit(oid)
        .context("the packfile did not contain the requested commit")?;
    let tree = commit.tree().context("the commit has no tree")?;

    let mut options = git2::build::CheckoutBuilder::new();
    options.force();
    repository
        .checkout_tree(tree.as_object(), Some(&mut options))
        .context("failed to check out the fetched tree")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(payload: &str) -> Vec<u8> {
        encode_pkt_line(payload)
    }

    #[test]
    fn encodes_pkt_line_length_including_the_header() {
        assert_eq!(encode_pkt_line("a\n"), b"0006a\n");
        assert_eq!(encode_pkt_line(""), b"0004");
    }

    #[test]
    fn parses_special_pkt_lines() {
        assert_eq!(next_pkt_line(b"0000").unwrap().0, PktLine::Flush);
        assert_eq!(next_pkt_line(b"0001").unwrap().0, PktLine::Delimiter);
        assert_eq!(next_pkt_line(b"0002").unwrap().0, PktLine::ResponseEnd);
        assert_eq!(
            next_pkt_line(b"0009hello").unwrap().0,
            PktLine::Data(b"hello")
        );
    }

    #[test]
    fn rejects_malformed_pkt_lines() {
        assert!(next_pkt_line(b"00").is_err(), "truncated header");
        assert!(next_pkt_line(b"zzzz").is_err(), "non-hex header");
        assert!(next_pkt_line(b"0003").is_err(), "reserved length");
        assert!(next_pkt_line(b"00ffshort").is_err(), "length past the end");
    }

    #[test]
    fn ls_refs_request_asks_only_for_the_named_branch() {
        let body = ls_refs_body("main");
        let text = String::from_utf8_lossy(&body).to_string();
        assert!(text.starts_with("0014command=ls-refs\n0001"));
        assert!(text.contains("ref-prefix refs/heads/main\n"));
        assert!(text.ends_with("0000"));
    }

    #[test]
    fn fetch_request_is_shallow_and_self_terminating() {
        let text = String::from_utf8_lossy(&fetch_body("abc")).to_string();
        assert!(text.contains("command=fetch\n"));
        assert!(text.contains("deepen 1\n"));
        assert!(text.contains("want abc\n"));
        assert!(text.contains("done\n"));
        assert!(text.ends_with("0000"));
    }

    #[test]
    fn finds_the_requested_branch_in_an_ls_refs_response() {
        let mut body = data("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa refs/heads/other\n");
        body.extend(data(
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb refs/heads/main\n",
        ));
        body.extend_from_slice(b"0000");

        assert_eq!(
            parse_ls_refs(&body, "main").unwrap(),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        );
    }

    #[test]
    fn ignores_trailing_ref_attributes() {
        let mut body =
            data("cccccccccccccccccccccccccccccccccccccccc refs/heads/main symref-target:HEAD\n");
        body.extend_from_slice(b"0000");
        assert_eq!(
            parse_ls_refs(&body, "main").unwrap(),
            "cccccccccccccccccccccccccccccccccccccccc"
        );
    }

    #[test]
    fn missing_branch_is_an_error_naming_the_branch() {
        let mut body = data("dddddddddddddddddddddddddddddddddddddddd refs/heads/main\n");
        body.extend_from_slice(b"0000");
        let error = parse_ls_refs(&body, "release").unwrap_err().to_string();
        assert!(error.contains("release"), "{error}");
    }

    #[test]
    fn extracts_pack_data_from_sideband_one() {
        let mut body = data("packfile\n");
        body.extend(data("\x01PACK-part-one"));
        body.extend(data("\x02resolving deltas"));
        body.extend(data("\x01-part-two"));
        body.extend_from_slice(b"0000");

        assert_eq!(extract_packfile(&body).unwrap(), b"PACK-part-one-part-two");
    }

    #[test]
    fn skips_sections_before_the_packfile() {
        let mut body = data("acknowledgments\n");
        body.extend(data("NAK\n"));
        body.extend_from_slice(b"0001");
        body.extend(data("packfile\n"));
        body.extend(data("\x01PACK"));
        body.extend_from_slice(b"0000");

        assert_eq!(extract_packfile(&body).unwrap(), b"PACK");
    }

    #[test]
    fn surfaces_remote_errors_from_sideband_three() {
        let mut body = data("packfile\n");
        body.extend(data("\x03repository not found"));
        body.extend_from_slice(b"0000");

        let error = extract_packfile(&body).unwrap_err().to_string();
        assert!(error.contains("repository not found"), "{error}");
    }

    #[test]
    fn empty_packfile_is_an_error() {
        let mut body = data("packfile\n");
        body.extend_from_slice(b"0000");
        assert!(extract_packfile(&body).is_err());
    }

    #[test]
    fn requires_protocol_v2() {
        let mut v2 = data("version 2\n");
        v2.extend(data("ls-refs=unborn\n"));
        v2.extend_from_slice(b"0000");
        assert!(ensure_protocol_v2(&v2).is_ok());

        let mut v1 = data("# service=git-upload-pack\n");
        v1.extend_from_slice(b"0000");
        assert!(ensure_protocol_v2(&v1).is_err());
    }

    #[test]
    fn endpoint_normalizes_trailing_slashes() {
        assert_eq!(
            endpoint("https://host/o/r.git/", "info/refs"),
            "https://host/o/r.git/info/refs"
        );
        assert_eq!(
            endpoint("https://host/o/r.git", "git-upload-pack"),
            "https://host/o/r.git/git-upload-pack"
        );
    }

    #[test]
    fn classifies_http_statuses() {
        use reqwest::StatusCode;
        assert_eq!(
            classify_status(StatusCode::UNAUTHORIZED),
            FetchFailure::Auth
        );
        assert_eq!(classify_status(StatusCode::FORBIDDEN), FetchFailure::Auth);
        assert_eq!(
            classify_status(StatusCode::NOT_FOUND),
            FetchFailure::NotFound
        );
        assert_eq!(
            classify_status(StatusCode::INTERNAL_SERVER_ERROR),
            FetchFailure::Unreachable
        );
    }

    #[test]
    fn control_and_pack_ceilings_are_ordered_and_overridable() {
        const { assert!(MAX_CONTROL_BYTES < DEFAULT_MAX_PACK_BYTES) };
        // Unset in tests, so the default applies.
        assert_eq!(max_pack_bytes(), DEFAULT_MAX_PACK_BYTES);
    }

    #[test]
    fn over_long_bodies_are_refused_rather_than_truncated() {
        // A body one byte past the ceiling must be an error, not a silently
        // truncated pkt-line stream that would then fail to parse.
        let server = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = server.local_addr().expect("addr");
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().expect("accept");
            let mut discard = [0u8; 1024];
            let _ = std::io::Read::read(&mut socket, &mut discard);
            let body = vec![b'x'; 64];
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut socket, response.as_bytes());
            let _ = std::io::Write::write_all(&mut socket, &body);
        });

        let response = reqwest::blocking::Client::new()
            .get(format!("http://{address}/"))
            .send()
            .expect("response");
        let error = read_capped(response, 16, "test body")
            .unwrap_err()
            .to_string();
        assert!(error.contains("over the 16 byte limit"), "{error}");

        handle.join().expect("server thread");
    }

    #[test]
    fn transport_errors_do_not_carry_the_request_url() {
        // A URL can embed credentials on memory git sources, and these errors
        // are logged for operators.
        let error = reqwest::blocking::Client::new()
            .get("http://127.0.0.1:1/secret-repo.git")
            .send()
            .expect_err("connection refused");
        assert!(error.to_string().contains("127.0.0.1:1"));

        let redacted = redact_url(error).to_string();
        assert!(!redacted.contains("127.0.0.1:1"), "{redacted}");
        assert!(!redacted.contains("secret-repo"), "{redacted}");
    }

    #[test]
    fn git_http_client_does_not_follow_redirects() {
        let destination = std::net::TcpListener::bind("127.0.0.1:0").expect("destination bind");
        destination
            .set_nonblocking(true)
            .expect("destination nonblocking");
        let destination_address = destination.local_addr().expect("destination addr");

        let redirector = std::net::TcpListener::bind("127.0.0.1:0").expect("redirector bind");
        let redirector_address = redirector.local_addr().expect("redirector addr");
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = redirector.accept().expect("redirect request");
            let mut discard = [0u8; 1024];
            let _ = std::io::Read::read(&mut socket, &mut discard);
            let response = format!(
                "HTTP/1.1 302 Found\r\nLocation: http://{destination_address}/internal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            std::io::Write::write_all(&mut socket, response.as_bytes()).expect("redirect response");
        });

        let response = client_builder()
            .no_proxy()
            .build()
            .expect("client")
            .get(format!("http://{redirector_address}/repo.git/info/refs"))
            .send()
            .expect("redirect response");
        assert_eq!(response.status(), reqwest::StatusCode::FOUND);
        assert!(
            matches!(destination.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "redirect destination must not receive a request"
        );

        handle.join().expect("redirector thread");
    }

    /// End-to-end against a real remote. Ignored by default because it needs
    /// network access; run with `--ignored` when changing the protocol code.
    #[test]
    #[ignore = "requires network access to github.com"]
    fn fetches_a_real_repository() {
        let checkout = tempfile::TempDir::new().expect("temp dir");
        shallow_checkout(
            &FetchRequest {
                url: "https://github.com/rust-lang/libc.git",
                branch: "main",
                auth_token: None,
            },
            checkout.path(),
        )
        .expect("checkout");

        assert!(checkout.path().join("Cargo.toml").is_file());
        assert!(checkout.path().join("src").is_dir());
    }
}
