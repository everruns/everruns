// Capabilities listing command

use crate::output::{OutputFormat, print_table_header, print_table_row};
use anyhow::Result;
use everruns_sdk::Everruns;

pub async fn run(client: &Everruns, output: OutputFormat, status_filter: &str) -> Result<()> {
    let response = client.capabilities().list().await?;

    // Filter by status
    let filtered: Vec<_> = response
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
        // For JSON/YAML output, project to a stable field set
        let items: Vec<_> = filtered
            .iter()
            .map(|c| {
                serde_json::json!({
                    "id": c.id,
                    "name": c.name,
                    "description": c.description,
                    "status": c.status,
                    "icon": c.icon,
                    "category": c.category,
                })
            })
            .collect();
        output.print_value(&serde_json::json!({ "data": items, "total": items.len() }));
    }

    Ok(())
}
