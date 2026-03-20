// Capabilities listing command
//
// TODO(sdk): Replace raw reqwest with SDK capabilities() methods once available.
// Note: Capabilities endpoint is not yet supported by the SDK,
// so we use reqwest directly here.

use crate::output::{OutputFormat, print_table_header, print_table_row};
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Capability info from API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub status: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListResponse<T> {
    data: Vec<T>,
}

pub async fn run(
    api_url: &str,
    api_key: &str,
    output: OutputFormat,
    status_filter: &str,
) -> Result<()> {
    let client = reqwest::Client::new();
    let url = format!("{}/v1/capabilities", api_url.trim_end_matches('/'));

    let response: ListResponse<CapabilityInfo> = client
        .get(&url)
        .header("Authorization", api_key)
        .send()
        .await?
        .json()
        .await?;

    // Filter by status
    let filtered: Vec<&CapabilityInfo> = response
        .data
        .iter()
        .filter(|c| {
            if status_filter == "all" {
                true
            } else {
                c.status == status_filter
            }
        })
        .collect();

    if output.is_text() {
        if filtered.is_empty() {
            println!("No capabilities found");
            return Ok(());
        }

        print_table_header(&[("ID", 22), ("NAME", 18), ("STATUS", 12), ("CATEGORY", 15)]);

        for cap in &filtered {
            let category = cap.category.as_deref().unwrap_or("-");
            print_table_row(&[
                (&cap.id, 22),
                (&cap.name, 18),
                (&cap.status, 12),
                (category, 15),
            ]);
        }
    } else {
        // For JSON/YAML output, return the filtered list
        output.print_value(&serde_json::json!({ "data": filtered, "total": filtered.len() }));
    }

    Ok(())
}
