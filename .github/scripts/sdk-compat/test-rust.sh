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

# The SDK session API changed across versions:
#   v0.1.0-v0.1.2: sessions().create(agent_id)
#   v0.1.3:        sessions().create(harness_id)
#   v0.1.4+:       sessions().create() (no args), use create_with_options() builder

# Compare versions using semver tuple comparison
version_cmp() {
  IFS='.' read -r maj1 min1 pat1 <<< "$1"
  IFS='.' read -r maj2 min2 pat2 <<< "$2"
  if [ "$maj1" -ne "$maj2" ]; then [ "$maj1" -gt "$maj2" ] && return 0 || return 1; fi
  if [ "$min1" -ne "$min2" ]; then [ "$min1" -gt "$min2" ] && return 0 || return 1; fi
  [ "$pat1" -ge "$pat2" ] && return 0 || return 1
}

if version_cmp "$version" "0.1.4"; then
# v0.1.4+: create() takes no args; use create_with_options() with builder
cat > "$workdir/smoke/src/main.rs" <<'RS'
use everruns_sdk::{Everruns, CreateSessionRequest};

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

    // 3. Create session with builder (harness_id optional in v0.1.4+)
    let harness_id = "harness_01933b5a000070008000000000000602";
    let req = CreateSessionRequest::new()
        .harness_id(harness_id)
        .agent_id(&agent.id);
    let session = client.sessions().create_with_options(req).await?;
    println!("  session created: {}", session.id);

    // 4. Fetch session and verify
    let fetched_session = client.sessions().get(&session.id).await?;
    assert_eq!(fetched_session.id, session.id, "session id mismatch");
    println!("  session fetch verified");

    println!("ok rust sdk {}", sdk_version);
    Ok(())
}
RS
elif version_cmp "$version" "0.1.3"; then
# v0.1.3: create(harness_id)
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
# v0.1.0-v0.1.2: create(agent_id)
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
