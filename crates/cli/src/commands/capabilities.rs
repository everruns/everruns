// Capabilities listing command

use crate::client::Client;
use crate::output::{print_table_header, print_table_row, OutputFormat};
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

pub async fn run(client: &Client, output: OutputFormat, status_filter: &str) -> Result<()> {
    let response: ListResponse<CapabilityInfo> = client.get("/v1/capabilities").await?;

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
        let output_data: Vec<&CapabilityInfo> = filtered;
        output.print_value(&serde_json::json!({ "data": output_data, "total": output_data.len() }));
    }

    Ok(())
}
