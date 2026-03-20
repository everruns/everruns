// `everruns files ls` — list remote session files.

use crate::commands::files::remote::RemoteClient;
use crate::output::{OutputFormat, print_table_header, print_table_row};
use anyhow::Result;

pub async fn run(
    api_url: &str,
    api_key: &str,
    output: OutputFormat,
    session_id: String,
    path: String,
    recursive: bool,
    long: bool,
) -> Result<()> {
    let client = RemoteClient::new(api_url, api_key, &session_id);
    let entries = client.list(&path, recursive).await?;

    if output.is_text() {
        if entries.is_empty() {
            println!("(empty)");
            return Ok(());
        }

        if long {
            print_table_header(&[("SIZE", 10), ("UPDATED", 20), ("PATH", 60)]);
            for entry in &entries {
                let size = if entry.is_directory {
                    "-".to_string()
                } else {
                    format_size(entry.size_bytes)
                };
                let updated = entry.updated_at.as_deref().unwrap_or("-");
                let path_display = if entry.is_directory {
                    format!("{}/", entry.path)
                } else {
                    entry.path.clone()
                };
                print_table_row(&[(&size, 10), (updated, 20), (&path_display, 60)]);
            }
        } else {
            for entry in &entries {
                if entry.is_directory {
                    println!("{}/", entry.path);
                } else {
                    println!("{}", entry.path);
                }
            }
        }
    } else {
        output.print_value(&serde_json::json!({ "entries": entries }));
    }

    Ok(())
}

fn format_size(bytes: i64) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size_bytes() {
        assert_eq!(format_size(0), "0B");
        assert_eq!(format_size(512), "512B");
        assert_eq!(format_size(1023), "1023B");
    }

    #[test]
    fn test_format_size_kilobytes() {
        assert_eq!(format_size(1024), "1.0K");
        assert_eq!(format_size(1536), "1.5K");
        assert_eq!(format_size(10240), "10.0K");
    }

    #[test]
    fn test_format_size_megabytes() {
        assert_eq!(format_size(1024 * 1024), "1.0M");
        assert_eq!(format_size(5 * 1024 * 1024), "5.0M");
    }
}
