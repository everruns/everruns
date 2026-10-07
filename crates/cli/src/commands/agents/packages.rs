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
            let response = super::super::api::ApiClient::new(api_url, api_key, org_id)
                .post_with_reason("/v1/agents/import", Some(&body), reason.as_deref())
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
            let response = super::super::api::ApiClient::new(api_url, api_key, org_id)
                .get(&format!("/v1/agents/{id}/export?format=json"))
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
            } else if export_format == "zip" {
                write_new(
                    out.as_ref().context("--out is required for ZIP export")?,
                    &package.to_zip()?,
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
                let response = super::super::api::ApiClient::new(api_url, api_key, org_id)
                    .post("/v1/agents/validate", Some(&body))
                    .await?;
                print_package_report(output, &response);
                anyhow::ensure!(response["valid"] == true, "Agent package validation failed");
            }
            Ok(())
        }
        AgentsCommand::Diff { file, target, .. } => {
            let package = everruns_core::agent_package::AgentPackage::load(&file)?;
            let body = serde_json::json!({"content":package.to_string(everruns_core::agent_package::Format::Json)?,"format":"json","target":target.context("provide --against PATH or --target AGENT_NAME")?});
            let response = super::super::api::ApiClient::new(api_url, api_key, org_id)
                .post("/v1/agents/diff", Some(&body))
                .await?;
            print_package_report(output, &response);
            Ok(())
        }
        _ => anyhow::bail!("expected a package command"),
    }
}
