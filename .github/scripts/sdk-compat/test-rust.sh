#!/usr/bin/env bash
set -euo pipefail

version="${1:?usage: test-rust.sh <version> <base_url> <api_key>}"
base_url="${2:?usage: test-rust.sh <version> <base_url> <api_key>}"
api_key="${3:?usage: test-rust.sh <version> <base_url> <api_key>}"

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

cargo new --quiet --bin "$workdir/smoke"

cat > "$workdir/smoke/Cargo.toml" <<TOML
[package]
name = "sdk-compat-rust"
version = "0.0.0"
edition = "2021"

[dependencies]
everruns-sdk = "=$version"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
TOML

# The SDK session API changed:
#   v0.1.0-v0.1.2: sessions().create(agent_id)
#   v0.1.3:        sessions().create(harness_id)
#   v0.1.4+:       sessions().create(), with CreateSessionRequest for options

version_gte_014() {
  IFS='.' read -r major minor patch <<< "$1"
  [ "$major" -gt 0 ] && return 0
  [ "$minor" -gt 1 ] && return 0
  [ "$minor" -eq 1 ] && [ "$patch" -ge 4 ] && return 0
  return 1
}

version_gte_013() {
  IFS='.' read -r major minor patch <<< "$1"
  [ "$major" -gt 0 ] && return 0
  [ "$minor" -gt 1 ] && return 0
  [ "$minor" -eq 1 ] && [ "$patch" -ge 3 ] && return 0
  return 1
}

if version_gte_014 "$version"; then
cat > "$workdir/smoke/src/main.rs" <<'RS'
use everruns_sdk::Everruns;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("EVERRUNS_API_KEY")?;
    let base_url = std::env::var("EVERRUNS_BASE_URL")?;
    let sdk_version = std::env::var("SDK_VERSION")?;

    let client = Everruns::with_base_url(api_key, &base_url)?;

    // 1. Create agent
    let agent = client.agents().create("sdk-compat-rs", "Compatibility test agent").await?;
    println!("  agent created: {}", agent.id);

    // 2. Fetch agent and verify
    let fetched = client.agents().get(&agent.id).await?;
    assert_eq!(fetched.id, agent.id, "agent id mismatch");
    println!("  agent fetch verified");

    // 3. Create session with server defaults
    let session = client.sessions().create().await?;
    println!("  session created: {}", session.id);

    // 4. Fetch session and verify
    let fetched_session = client.sessions().get(&session.id).await?;
    assert_eq!(fetched_session.id, session.id, "session id mismatch");
    println!("  session fetch verified");

    println!("ok rust sdk {}", sdk_version);
    Ok(())
}
RS
elif version_gte_013 "$version"; then
cat > "$workdir/smoke/src/main.rs" <<'RS'
use everruns_sdk::Everruns;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("EVERRUNS_API_KEY")?;
    let base_url = std::env::var("EVERRUNS_BASE_URL")?;
    let sdk_version = std::env::var("SDK_VERSION")?;

    let client = Everruns::with_base_url(api_key, &base_url)?;

    // 1. Create agent
    let agent = client.agents().create("sdk-compat-rs", "Compatibility test agent").await?;
    println!("  agent created: {}", agent.id);

    // 2. Fetch agent and verify
    let fetched = client.agents().get(&agent.id).await?;
    assert_eq!(fetched.id, agent.id, "agent id mismatch");
    println!("  agent fetch verified");

    // 3. Create session with the well-known Generic harness (seeded in every org)
    let harness_id = "harness_01933b5a000070008000000000000602";
    let session = client.sessions().create(harness_id).await?;
    println!("  session created: {}", session.id);

    // 4. Fetch session and verify
    let fetched_session = client.sessions().get(&session.id).await?;
    assert_eq!(fetched_session.id, session.id, "session id mismatch");
    println!("  session fetch verified");

    println!("ok rust sdk {}", sdk_version);
    Ok(())
}
RS
else
cat > "$workdir/smoke/src/main.rs" <<'RS'
use everruns_sdk::Everruns;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("EVERRUNS_API_KEY")?;
    let base_url = std::env::var("EVERRUNS_BASE_URL")?;
    let sdk_version = std::env::var("SDK_VERSION")?;

    let client = Everruns::with_base_url(api_key, &base_url)?;

    // 1. Create agent
    let agent = client.agents().create("sdk-compat-rs", "Compatibility test agent").await?;
    println!("  agent created: {}", agent.id);

    // 2. Fetch agent and verify
    let fetched = client.agents().get(&agent.id).await?;
    assert_eq!(fetched.id, agent.id, "agent id mismatch");
    println!("  agent fetch verified");

    // 3. Create session (old API: takes agent_id, not harness_id)
    let session = client.sessions().create(&agent.id).await?;
    println!("  session created: {}", session.id);

    // 4. Fetch session and verify
    let fetched_session = client.sessions().get(&session.id).await?;
    assert_eq!(fetched_session.id, session.id, "session id mismatch");
    println!("  session fetch verified");

    println!("ok rust sdk {}", sdk_version);
    Ok(())
}
RS
fi

(
  cd "$workdir/smoke"
  EVERRUNS_API_KEY="$api_key" \
  EVERRUNS_BASE_URL="$base_url" \
  SDK_VERSION="$version" \
  cargo run --quiet
)
