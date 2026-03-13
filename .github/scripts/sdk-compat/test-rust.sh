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
rand = "0.9"
TOML

cat > "$workdir/smoke/src/main.rs" <<'RS'
use everruns_sdk::{Client, CreateAgentRequest, CreateSessionRequest};
use rand::{distr::Alphanumeric, Rng};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("EVERRUNS_API_KEY")?;
    let base_url = std::env::var("EVERRUNS_BASE_URL")?;

    let client = Client::builder().api_key(api_key).base_url(base_url).build()?;

    let suffix: String = rand::rng()
        .sample_iter(Alphanumeric)
        .take(8)
        .map(char::from)
        .collect();

    let agent = client
        .agents()
        .create(CreateAgentRequest {
            name: format!("sdk-compat-rs-{suffix}"),
            system_prompt: "Compatibility test agent".to_string(),
            ..Default::default()
        })
        .await?;

    let fetched_agent = client.agents().get(&agent.id).await?;
    if fetched_agent.id != agent.id {
        return Err(format!("agent id mismatch: {} != {}", fetched_agent.id, agent.id).into());
    }

    let session = client
        .sessions()
        .create(CreateSessionRequest {
            agent_id: agent.id.clone(),
            ..Default::default()
        })
        .await?;

    let fetched_session = client.sessions().get(&session.id).await?;
    if fetched_session.id != session.id {
        return Err(format!("session id mismatch: {} != {}", fetched_session.id, session.id).into());
    }

    println!("ok rust sdk {}", std::env::var("SDK_VERSION")?);
    Ok(())
}
RS

(
  cd "$workdir/smoke"
  EVERRUNS_API_KEY="$api_key" \
  EVERRUNS_BASE_URL="$base_url" \
  SDK_VERSION="$version" \
  cargo run --quiet
)
