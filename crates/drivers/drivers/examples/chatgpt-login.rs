//! Login helper for a remote self-hosted installation. Credentials stay in a private file.
use everruns_drivers::chatgpt::{ChatGptRegistration, login::LoginAttempt, oauth::Endpoints};
use std::{io::Write, path::PathBuf};
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut host_id = None;
    let mut output = None;
    let mut installation = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--host-id" => host_id = args.next(),
            "--installation" => installation = args.next().map(PathBuf::from),
            "--output" => output = args.next().map(PathBuf::from),
            _ => anyhow::bail!(
                "Usage: chatgpt-login --installation <downloaded setup file> --output <private login file>"
            ),
        }
    }
    let setup: serde_json::Value = if let Some(path) = installation {
        serde_json::from_slice(&std::fs::read(path)?)?
    } else {
        serde_json::Value::Null
    };
    let host_id = host_id
        .or_else(|| setup["host_id"].as_str().map(str::to_owned))
        .ok_or_else(|| anyhow::anyhow!("--installation or --host-id is required"))?;
    let registration: Option<ChatGptRegistration> = setup
        .get("registration")
        .filter(|v| !v.is_null())
        .map(|v| serde_json::from_value(v.clone()))
        .transpose()?;
    let output = output.ok_or_else(|| anyhow::anyhow!("--output is required"))?;
    let attempt = LoginAttempt::start(
        Endpoints::production(),
        "Everruns",
        &host_id,
        registration,
        None,
        false,
    )
    .await?;
    let nonce = attempt.nonce().to_string();
    eprintln!("Open this URL on this computer:\n{}", attempt.authorize_url);
    let auth = attempt.finish().await?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(output)?;
    file.write_all(&serde_json::to_vec(
        &serde_json::json!({"auth":auth,"nonce":nonce,"host_id":host_id}),
    )?)?;
    file.sync_all()?;
    eprintln!("Login saved. Upload the private file in Everruns, then remove it.");
    Ok(())
}
