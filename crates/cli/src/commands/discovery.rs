use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::Value;

use crate::commands::api::ApiClient;
use crate::output::OutputFormat;

#[derive(Subcommand, Debug)]
pub enum DiscoveryCommands {
    /// List resources
    List,
    /// Get a resource by ID
    Get { id: String },
}

#[derive(Clone, Copy)]
pub struct Resource {
    pub path: &'static str,
    pub collection_key: &'static str,
    pub empty_message: &'static str,
    pub columns: &'static [(&'static str, &'static str)],
    pub detail_fields: &'static [(&'static str, &'static str)],
}

pub async fn run(
    command: DiscoveryCommands,
    api_url: &str,
    api_key: &str,
    org_id: Option<&str>,
    format: OutputFormat,
    resource: Resource,
) -> Result<()> {
    let client = ApiClient::new(api_url, api_key, org_id);
    match command {
        DiscoveryCommands::List => {
            let response: Value = client.get(resource.path).await?;
            if format.is_text() {
                let items = response[resource.collection_key]
                    .as_array()
                    .with_context(|| {
                        format!("API response is missing `{}`", resource.collection_key)
                    })?;
                print_items(items, resource.columns, resource.empty_message);
            } else {
                format.print_value(&response);
            }
        }
        DiscoveryCommands::Get { id } => {
            let response: Value = client.get(&resource_path(resource.path, &id)).await?;
            if format.is_text() {
                print_fields(&response, resource.detail_fields);
            } else {
                format.print_value(&response);
            }
        }
    }
    Ok(())
}

pub(super) fn resource_path(collection: &str, id: &str) -> String {
    format!("{collection}/{}", urlencoding::encode(id))
}

fn print_items(items: &[Value], columns: &[(&str, &str)], empty_message: &str) {
    if items.is_empty() {
        println!("{empty_message}");
        return;
    }

    let rows: Vec<Vec<String>> = items
        .iter()
        .map(|item| {
            columns
                .iter()
                .map(|(_, field)| display_value(item.get(*field)))
                .collect()
        })
        .collect();

    let widths: Vec<usize> = columns
        .iter()
        .enumerate()
        .map(|(column_index, (heading, _))| {
            rows.iter()
                .map(|row| row[column_index].len())
                .max()
                .unwrap_or(0)
                .max(heading.len())
        })
        .collect();

    for (index, (heading, _)) in columns.iter().enumerate() {
        if index > 0 {
            print!("  ");
        }
        print!("{heading:<width$}", width = widths[index]);
    }
    println!();

    for row in rows {
        for (index, value) in row.iter().enumerate() {
            if index > 0 {
                print!("  ");
            }
            print!("{value:<width$}", width = widths[index]);
        }
        println!();
    }
}

fn print_fields(value: &Value, fields: &[(&str, &str)]) {
    for (label, field) in fields {
        if let Some(value) = value.get(*field) {
            println!("{label}: {}", display_value(Some(value)));
        }
    }
}

fn display_value(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "-".to_string(),
        Some(Value::String(value)) => value.clone(),
        Some(value) => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::resource_path;

    #[test]
    fn resource_path_encodes_identifier_as_one_segment() {
        assert_eq!(
            resource_path("/skills", "author/name with spaces"),
            "/skills/author%2Fname%20with%20spaces"
        );
    }
}
