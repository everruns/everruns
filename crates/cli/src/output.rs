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
