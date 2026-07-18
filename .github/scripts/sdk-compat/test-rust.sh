#!/usr/bin/env bash
set -euo pipefail

version="${1:?usage: test-rust.sh <version> <base_url> <api_key>}"
base_url="${2:?usage: test-rust.sh <version> <base_url> <api_key>}"
api_key="${3:?usage: test-rust.sh <version> <base_url> <api_key>}"

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

# Resolve the Generic harness by name — UUIDs are assigned at DB seed time
# and are no longer stable across deployments. Built-in harness seeding
# happens after server startup, so wait for it explicitly instead of assuming
# /health implies the harness is already queryable.
harness_id="$("$(dirname "$0")/resolve-harness-id.sh" "$base_url" "$api_key")"

cargo new --quiet --bin "$workdir/smoke"

cat > "$workdir/smoke/Cargo.toml" <<TOML
[package]
name = "sdk-compat-rust"
version = "0.0.0"
edition = "2021"

[dependencies]
everruns-sdk = "=$version"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
reqwest = { version = "0.12", features = ["json"] }
serde_json = "1"
TOML

cat > "$workdir/smoke/src/compat.rs" <<'RS'
use everruns_sdk::Everruns;

pub async fn exercise_new_surfaces(
    client: &Everruns,
    base_url: &str,
    api_key: &str,
    agent_id: &str,
    session_id: &str,
    suffix: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let http = reqwest::Client::new();
    let trigger: serde_json::Value = http
        .post(format!("{}/v1/agents/{agent_id}/triggers", base_url.trim_end_matches('/')))
        .bearer_auth(api_key)
        .json(&serde_json::json!({
            "cron_expression": "0 0 * * *",
            "timezone": "UTC",
            "session_mode": "session_per_invocation",
            "message": "SDK compatibility trigger",
            "enabled": false
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(trigger["agent_id"], agent_id);
    println!("  trigger created: {}", trigger["id"]);

    let guest = client
        .agents()
        .create(
            &format!("sdk-compat-rs-guest-{suffix}"),
            "Compatibility guest agent",
        )
        .await?;
    let participant: serde_json::Value = http
        .post(format!(
            "{}/v1/sessions/{session_id}/participants",
            base_url.trim_end_matches('/')
        ))
        .bearer_auth(api_key)
        .json(&serde_json::json!({ "kind": "agent", "agent_id": guest.id.clone() }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(participant["agent_id"], guest.id);
    println!("  participant added: {}", participant["id"]);
    Ok(())
}
RS

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
mod compat;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("EVERRUNS_API_KEY")?;
    let base_url = std::env::var("EVERRUNS_BASE_URL")?;
    let sdk_version = std::env::var("SDK_VERSION")?;
    let suffix = format!("{}-{}", sdk_version.replace('.', "-"), std::process::id());

    let client = Everruns::with_base_url(api_key.clone(), &base_url)?;

    // 1. Create agent
    let agent = client.agents().create(&format!("sdk-compat-rs-{suffix}"), "Compatibility test agent").await?;
    println!("  agent created: {}", agent.id);

    // 2. Fetch agent and verify
    let fetched = client.agents().get(&agent.id).await?;
    assert_eq!(fetched.id, agent.id, "agent id mismatch");
    println!("  agent fetch verified");

    // 3. Create session with builder (harness_id optional in v0.1.4+)
    let harness_id = std::env::var("EVERRUNS_HARNESS_ID")?;
    let req = CreateSessionRequest::new()
        .harness_id(&harness_id)
        .agent_id(&agent.id);
    let session = client.sessions().create_with_options(req).await?;
    println!("  session created: {}", session.id);

    // 4. Fetch session and verify
    let fetched_session = client.sessions().get(&session.id).await?;
    assert_eq!(fetched_session.id, session.id, "session id mismatch");
    println!("  session fetch verified");

    // 5. Raw HTTP keeps older published SDK cohorts compatible while checking
    // the current trigger and participant API surfaces.
    compat::exercise_new_surfaces(
        &client, &base_url, &api_key, &agent.id, &session.id, &suffix,
    )
    .await?;

    println!("ok rust sdk {}", sdk_version);
    Ok(())
}
RS
elif version_cmp "$version" "0.1.3"; then
# v0.1.3: create(harness_id)
cat > "$workdir/smoke/src/main.rs" <<'RS'
use everruns_sdk::Everruns;
mod compat;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("EVERRUNS_API_KEY")?;
    let base_url = std::env::var("EVERRUNS_BASE_URL")?;
    let sdk_version = std::env::var("SDK_VERSION")?;
    let suffix = format!("{}-{}", sdk_version.replace('.', "-"), std::process::id());

    let client = Everruns::with_base_url(api_key.clone(), &base_url)?;

    // 1. Create agent
    let agent = client.agents().create(&format!("sdk-compat-rs-{suffix}"), "Compatibility test agent").await?;
    println!("  agent created: {}", agent.id);

    // 2. Fetch agent and verify
    let fetched = client.agents().get(&agent.id).await?;
    assert_eq!(fetched.id, agent.id, "agent id mismatch");
    println!("  agent fetch verified");

    // 3. Create session with the Generic harness (resolved by name at runtime)
    let harness_id = std::env::var("EVERRUNS_HARNESS_ID")?;
    let session = client.sessions().create(&harness_id).await?;
    println!("  session created: {}", session.id);

    // 4. Fetch session and verify
    let fetched_session = client.sessions().get(&session.id).await?;
    assert_eq!(fetched_session.id, session.id, "session id mismatch");
    println!("  session fetch verified");

    compat::exercise_new_surfaces(
        &client, &base_url, &api_key, &agent.id, &session.id, &suffix,
    )
    .await?;

    println!("ok rust sdk {}", sdk_version);
    Ok(())
}
RS
else
# v0.1.0-v0.1.2: create(agent_id)
cat > "$workdir/smoke/src/main.rs" <<'RS'
use everruns_sdk::Everruns;
mod compat;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("EVERRUNS_API_KEY")?;
    let base_url = std::env::var("EVERRUNS_BASE_URL")?;
    let sdk_version = std::env::var("SDK_VERSION")?;
    let suffix = format!("{}-{}", sdk_version.replace('.', "-"), std::process::id());

    let client = Everruns::with_base_url(api_key.clone(), &base_url)?;

    // 1. Create agent
    let agent = client.agents().create(&format!("sdk-compat-rs-{suffix}"), "Compatibility test agent").await?;
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

    compat::exercise_new_surfaces(
        &client, &base_url, &api_key, &agent.id, &session.id, &suffix,
    )
    .await?;

    println!("ok rust sdk {}", sdk_version);
    Ok(())
}
RS
fi

(
  cd "$workdir/smoke"
  EVERRUNS_API_KEY="$api_key" \
  EVERRUNS_BASE_URL="$base_url" \
  EVERRUNS_HARNESS_ID="$harness_id" \
  SDK_VERSION="$version" \
  cargo run --quiet
)
