// Output formatting for CLI

use serde::Serialize;

#[derive(Clone, Copy)]
pub enum OutputFormat {
    Text,
    Json,
    Yaml,
}

impl OutputFormat {
    pub fn from_str(s: &str) -> Self {
        match s {
            "json" => OutputFormat::Json,
            "yaml" => OutputFormat::Yaml,
            _ => OutputFormat::Text,
        }
    }

    pub fn print_value<T: Serialize>(&self, value: &T) {
        match self {
            OutputFormat::Json => {
                println!("{}", serde_json::to_string_pretty(value).unwrap());
            }
            OutputFormat::Yaml => {
                println!("{}", serde_yaml::to_string(value).unwrap());
            }
            OutputFormat::Text => {
                // Text format is handled by each command
            }
        }
    }

    pub fn is_text(&self) -> bool {
        matches!(self, OutputFormat::Text)
    }
}

/// Print a simple key-value pair for text output
pub fn print_field(label: &str, value: &str) {
    println!("{:<14} {}", format!("{}:", label), value);
}

/// Print a table header
pub fn print_table_header(columns: &[(&str, usize)]) {
    let header: String = columns
        .iter()
        .map(|(name, width)| format!("{:<width$}", name, width = width))
        .collect::<Vec<_>>()
        .join("  ");
    println!("{}", header);
}

/// Print a table row
pub fn print_table_row(values: &[(&str, usize)]) {
    let row: String = values
        .iter()
        .map(|(val, width)| {
            let s = if val.len() > *width {
                format!("{}...", &val[..(width - 3)])
            } else {
                val.to_string()
            };
            format!("{:<width$}", s, width = width)
        })
        .collect::<Vec<_>>()
        .join("  ");
    println!("{}", row);
}

/// Format a table row (returns string instead of printing)
#[cfg(test)]
fn format_table_row(values: &[(&str, usize)]) -> String {
    values
        .iter()
        .map(|(val, width)| {
            let s = if val.len() > *width {
                format!("{}...", &val[..(width - 3)])
            } else {
                val.to_string()
            };
            format!("{:<width$}", s, width = width)
        })
        .collect::<Vec<_>>()
        .join("  ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_format_from_str() {
        assert!(matches!(OutputFormat::from_str("json"), OutputFormat::Json));
        assert!(matches!(OutputFormat::from_str("yaml"), OutputFormat::Yaml));
        assert!(matches!(OutputFormat::from_str("text"), OutputFormat::Text));
        assert!(matches!(OutputFormat::from_str("unknown"), OutputFormat::Text));
        assert!(matches!(OutputFormat::from_str(""), OutputFormat::Text));
    }

    #[test]
    fn test_output_format_is_text() {
        assert!(OutputFormat::Text.is_text());
        assert!(!OutputFormat::Json.is_text());
        assert!(!OutputFormat::Yaml.is_text());
    }

    #[test]
    fn test_format_table_row_normal() {
        let row = format_table_row(&[("hello", 10), ("world", 10)]);
        assert_eq!(row, "hello       world     ");
    }

    #[test]
    fn test_format_table_row_truncation() {
        let row = format_table_row(&[("verylongtext", 8)]);
        assert_eq!(row, "veryl...");
    }

    #[test]
    fn test_format_table_row_exact_width() {
        let row = format_table_row(&[("exact", 5)]);
        assert_eq!(row, "exact");
    }
}
