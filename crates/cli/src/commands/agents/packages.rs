//! Local and remote portable package commands.
use super::{AgentsCommand, OutputFormat, print_field};
use anyhow::{Context, Result};
use std::path::Path;

fn parse_format(format: &str) -> Result<everruns_core::agent_package::Format> {
    use everruns_core::agent_package::Format;
    match format {
        "auto" => Ok(Format::Auto),
        "markdown" | "md" => Ok(Format::Markdown),
        "toml" => Ok(Format::Toml),
        "yaml" | "yml" => Ok(Format::Yaml),
        "json" => Ok(Format::Json),
        _ => anyhow::bail!("Unsupported format {format}"),
    }
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| {
            format!(
                "Cannot create {} (existing files are not overwritten)",
                path.display()
            )
        })?;
    file.write_all(bytes)?;
    Ok(())
}

pub(super) fn print_package_report(output: OutputFormat, value: &serde_json::Value) {
    if output.is_text() {
        if let Some(name) = value.get("name").and_then(serde_json::Value::as_str)
            && value.get("id").is_some()
        {
            print_field("Agent", name);
        } else {
            OutputFormat::Json.print_value(value);
        }
    } else {
        output.print_value(value);
    }
}

/// Offline validation and local diff never require platform credentials.
pub fn run_local(command: &AgentsCommand, output: OutputFormat) -> Option<Result<()>> {
    match command {
        AgentsCommand::Validate {
            file,
            remote: false,
        } => Some((|| {
            match everruns_core::agent_package::AgentPackage::load(file) {
                Ok(package) => print_package_report(
                    output,
                    &serde_json::json!({"valid":true,"name":package.manifest.name,"files":package.files()?.len(),"diagnostics":[]}),
                ),
                Err(error) => {
                    print_package_report(
                        output,
                        &serde_json::json!({"valid":false,"diagnostics":error.0}),
                    );
                    anyhow::bail!("Agent package validation failed");
                }
            }
            Ok(())
        })()),
        AgentsCommand::Diff {
            file,
            against: Some(against),
            ..
        } => Some((|| {
            let before = everruns_core::agent_package::AgentPackage::load(against)?;
            let proposed = everruns_core::agent_package::AgentPackage::load(file)?;
            let changes = before.diff(&proposed)?;
            print_package_report(
                output,
                &serde_json::json!({"changed":!changes.is_empty(),"changes":changes}),
            );
            Ok(())
        })()),
        _ => None,
    }
}

pub(super) async fn run(
    command: AgentsCommand,
    api_url: &str,
    api_key: &str,
    org_id: Option<&str>,
    output: OutputFormat,
) -> Result<()> {
    match command {
        AgentsCommand::Import {
            file,
            content,
            target,
            format: input_format,
            reason,
        } => {
            let package = if let Some(content) = content {
                everruns_core::agent_package::AgentPackage::parse(
                    &content,
                    parse_format(input_format.as_deref().unwrap_or("auto"))?,
                )?
            } else {
                everruns_core::agent_package::AgentPackage::load(file.as_deref().unwrap_or("."))?
            };
            let body = serde_json::json!({"content":package.to_string(everruns_core::agent_package::Format::Json)?,"format":"json","target":target});
            let response = super::super::api::ApiClient::connect(api_url, api_key, org_id)
                .await?
                .post_with_reason("/v1/platform/agents/import", Some(&body), reason.as_deref())
                .await?;
            print_package_report(output, &response);
            Ok(())
        }
        AgentsCommand::Export {
            agent,
            format: export_format,
            out,
        } => {
            let id = urlencoding::encode(&agent);
            let client = super::super::api::ApiClient::connect(api_url, api_key, org_id).await?;
            if export_format == "zip" {
                let path = out.as_ref().context("--out is required for ZIP export")?;
                let response = client
                    .request(
                        reqwest::Method::GET,
                        &format!("/v1/platform/agents/{id}/export?format=zip"),
                    )
                    .send()
                    .await?;
                if !response.status().is_success() {
                    return Err(super::super::api::ApiError::from_response(
                        "Export agent ZIP",
                        response,
                    )
                    .await
                    .into());
                }
                write_new(path, &response.bytes().await?)?;
                return Ok(());
            }
            let response = client
                .get(&format!("/v1/platform/agents/{id}/export?format=json"))
                .await?;
            let package = everruns_core::agent_package::AgentPackage::parse(
                &response.to_string(),
                everruns_core::agent_package::Format::Json,
            )?;
            if export_format == "folder" {
                package.write_folder(
                    out.as_ref()
                        .context("--out is required for folder export")?,
                )?;
            } else {
                let text = package.to_string(parse_format(&export_format)?)?;
                if let Some(path) = out {
                    write_new(&path, text.as_bytes())?;
                } else {
                    print!("{text}");
                }
            }
            Ok(())
        }
        AgentsCommand::Validate { file, remote } => {
            let package = everruns_core::agent_package::AgentPackage::load(&file)?;
            if remote {
                let body = serde_json::json!({"content":package.to_string(everruns_core::agent_package::Format::Json)?,"format":"json"});
                let response = super::super::api::ApiClient::connect(api_url, api_key, org_id)
                    .await?
                    .post("/v1/platform/agents/validate", Some(&body))
                    .await?;
                print_package_report(output, &response);
                anyhow::ensure!(response["valid"] == true, "Agent package validation failed");
            }
            Ok(())
        }
        AgentsCommand::Diff { file, target, .. } => {
            let package = everruns_core::agent_package::AgentPackage::load(&file)?;
            let body = serde_json::json!({"content":package.to_string(everruns_core::agent_package::Format::Json)?,"format":"json","target":target.context("provide --against PATH or --target AGENT_NAME")?});
            let response = super::super::api::ApiClient::connect(api_url, api_key, org_id)
                .await?
                .post("/v1/platform/agents/diff", Some(&body))
                .await?;
            print_package_report(output, &response);
            Ok(())
        }
        _ => anyhow::bail!("expected a package command"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::api::capture::serve_once;

    #[tokio::test]
    async fn zip_export_downloads_the_server_archive_with_org_and_auth() {
        const ARCHIVE: &str = "PK\u{3}\u{4}server-bundled-team";
        let (url, captured) = serve_once(200, ARCHIVE).await;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("team.zip");
        run(
            AgentsCommand::Export {
                agent: "coordinator".into(),
                format: "zip".into(),
                out: Some(path.clone()),
            },
            &url,
            "test-key",
            Some("org-test"),
            OutputFormat::Json,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(path).unwrap(), ARCHIVE.as_bytes());
        let request = captured.await.unwrap();
        assert!(request.headers[0].contains("/agents/coordinator/export?format=zip"));
        assert_eq!(
            request.header("authorization").as_deref(),
            Some("Bearer test-key")
        );
        assert_eq!(request.header("x-org-id").as_deref(), Some("org-test"));
    }

    #[tokio::test]
    async fn zip_export_error_does_not_create_an_archive() {
        let (url, captured) = serve_once(400, r#"{"detail":"Cannot bundle team agent"}"#).await;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("team.zip");
        let error = run(
            AgentsCommand::Export {
                agent: "coordinator".into(),
                format: "zip".into(),
                out: Some(path.clone()),
            },
            &url,
            "test-key",
            None,
            OutputFormat::Json,
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("Cannot bundle team agent"));
        assert!(!path.exists());
        captured.await.unwrap();
    }
}
