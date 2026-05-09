use std::collections::BTreeMap;

use everruns_core::reporting::{
    DatasetCatalog, DatasetCatalogEntry, ReportFilterOp, ReportOrderBy, ReportQuery,
};

use crate::domains::common::CommandError;

pub const MAX_REPORT_LIMIT: u32 = 1_000;
const MAX_DIMENSIONS: usize = 8;
const MAX_MEASURES: usize = 16;
const MAX_FILTERS: usize = 20;
const MAX_RANGE_DAYS: i64 = 370;

#[derive(Debug, Clone, Copy)]
pub struct FieldSpec {
    pub name: &'static str,
    pub sql: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct MeasureSpec {
    pub name: &'static str,
    pub sql: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct DatasetSpec {
    pub name: &'static str,
    pub table: &'static str,
    pub dimensions: &'static [FieldSpec],
    pub measures: &'static [MeasureSpec],
    pub filters: &'static [FieldSpec],
}

const LLM_DIMS: &[FieldSpec] = &[
    FieldSpec {
        name: "session",
        sql: "session_id",
    },
    FieldSpec {
        name: "turn",
        sql: "turn_id",
    },
    FieldSpec {
        name: "user",
        sql: "user_id",
    },
    FieldSpec {
        name: "principal",
        sql: "principal_id",
    },
    FieldSpec {
        name: "agent",
        sql: "agent_id",
    },
    FieldSpec {
        name: "agent_name",
        sql: "agent_name_snapshot",
    },
    FieldSpec {
        name: "harness",
        sql: "harness_id",
    },
    FieldSpec {
        name: "harness_name",
        sql: "harness_name_snapshot",
    },
    FieldSpec {
        name: "app",
        sql: "app_id",
    },
    FieldSpec {
        name: "blueprint",
        sql: "blueprint_id",
    },
    FieldSpec {
        name: "day",
        sql: "time_bucket_day",
    },
    FieldSpec {
        name: "model",
        sql: "model",
    },
    FieldSpec {
        name: "provider",
        sql: "provider",
    },
    FieldSpec {
        name: "finish_reason",
        sql: "finish_reason",
    },
];

const TOOL_DIMS: &[FieldSpec] = &[
    FieldSpec {
        name: "session",
        sql: "session_id",
    },
    FieldSpec {
        name: "turn",
        sql: "turn_id",
    },
    FieldSpec {
        name: "user",
        sql: "user_id",
    },
    FieldSpec {
        name: "principal",
        sql: "principal_id",
    },
    FieldSpec {
        name: "agent",
        sql: "agent_id",
    },
    FieldSpec {
        name: "agent_name",
        sql: "agent_name_snapshot",
    },
    FieldSpec {
        name: "harness",
        sql: "harness_id",
    },
    FieldSpec {
        name: "harness_name",
        sql: "harness_name_snapshot",
    },
    FieldSpec {
        name: "app",
        sql: "app_id",
    },
    FieldSpec {
        name: "blueprint",
        sql: "blueprint_id",
    },
    FieldSpec {
        name: "day",
        sql: "time_bucket_day",
    },
    FieldSpec {
        name: "tool",
        sql: "tool_name",
    },
    FieldSpec {
        name: "tool_display_name",
        sql: "tool_display_name_snapshot",
    },
    FieldSpec {
        name: "tool_status",
        sql: "tool_status",
    },
    FieldSpec {
        name: "capability",
        sql: "capability_id",
    },
    FieldSpec {
        name: "capability_name",
        sql: "capability_name_snapshot",
    },
];

const TURN_DIMS: &[FieldSpec] = &[
    FieldSpec {
        name: "session",
        sql: "session_id",
    },
    FieldSpec {
        name: "turn",
        sql: "turn_id",
    },
    FieldSpec {
        name: "user",
        sql: "user_id",
    },
    FieldSpec {
        name: "principal",
        sql: "principal_id",
    },
    FieldSpec {
        name: "agent",
        sql: "agent_id",
    },
    FieldSpec {
        name: "agent_name",
        sql: "agent_name_snapshot",
    },
    FieldSpec {
        name: "harness",
        sql: "harness_id",
    },
    FieldSpec {
        name: "harness_name",
        sql: "harness_name_snapshot",
    },
    FieldSpec {
        name: "app",
        sql: "app_id",
    },
    FieldSpec {
        name: "blueprint",
        sql: "blueprint_id",
    },
    FieldSpec {
        name: "day",
        sql: "time_bucket_day",
    },
    FieldSpec {
        name: "turn_status",
        sql: "turn_status",
    },
];

const SESSION_DIMS: &[FieldSpec] = &[
    FieldSpec {
        name: "session",
        sql: "session_id",
    },
    FieldSpec {
        name: "user",
        sql: "user_id",
    },
    FieldSpec {
        name: "principal",
        sql: "principal_id",
    },
    FieldSpec {
        name: "agent",
        sql: "agent_id",
    },
    FieldSpec {
        name: "agent_name",
        sql: "agent_name_snapshot",
    },
    FieldSpec {
        name: "harness",
        sql: "harness_id",
    },
    FieldSpec {
        name: "harness_name",
        sql: "harness_name_snapshot",
    },
    FieldSpec {
        name: "app",
        sql: "app_id",
    },
    FieldSpec {
        name: "blueprint",
        sql: "blueprint_id",
    },
    FieldSpec {
        name: "day",
        sql: "time_bucket_day",
    },
    FieldSpec {
        name: "session_status",
        sql: "session_status",
    },
];

const BUDGET_DIMS: &[FieldSpec] = &[
    FieldSpec {
        name: "session",
        sql: "session_id",
    },
    FieldSpec {
        name: "user",
        sql: "user_id",
    },
    FieldSpec {
        name: "principal",
        sql: "principal_id",
    },
    FieldSpec {
        name: "agent",
        sql: "agent_id",
    },
    FieldSpec {
        name: "agent_name",
        sql: "agent_name_snapshot",
    },
    FieldSpec {
        name: "harness",
        sql: "harness_id",
    },
    FieldSpec {
        name: "harness_name",
        sql: "harness_name_snapshot",
    },
    FieldSpec {
        name: "app",
        sql: "app_id",
    },
    FieldSpec {
        name: "blueprint",
        sql: "blueprint_id",
    },
    FieldSpec {
        name: "day",
        sql: "time_bucket_day",
    },
    FieldSpec {
        name: "budget",
        sql: "budget_id",
    },
    FieldSpec {
        name: "budget_subject_type",
        sql: "budget_subject_type",
    },
    FieldSpec {
        name: "budget_subject",
        sql: "budget_subject_id",
    },
    FieldSpec {
        name: "currency",
        sql: "currency",
    },
    FieldSpec {
        name: "meter_source",
        sql: "meter_source",
    },
    FieldSpec {
        name: "ref_type",
        sql: "ref_type",
    },
];

const LLM_MEASURES: &[MeasureSpec] = &[
    MeasureSpec {
        name: "input_tokens",
        sql: "COALESCE(SUM(input_tokens), 0)",
    },
    MeasureSpec {
        name: "output_tokens",
        sql: "COALESCE(SUM(output_tokens), 0)",
    },
    MeasureSpec {
        name: "cache_read_tokens",
        sql: "COALESCE(SUM(cache_read_tokens), 0)",
    },
    MeasureSpec {
        name: "cache_creation_tokens",
        sql: "COALESCE(SUM(cache_creation_tokens), 0)",
    },
    MeasureSpec {
        name: "total_tokens",
        sql: "COALESCE(SUM(total_tokens), 0)",
    },
    MeasureSpec {
        name: "duration_ms",
        sql: "COALESCE(SUM(duration_ms), 0)",
    },
    MeasureSpec {
        name: "avg_duration_ms",
        sql: "COALESCE(AVG(NULLIF(duration_ms, 0)), 0)",
    },
    MeasureSpec {
        name: "time_to_first_token_ms",
        sql: "COALESCE(SUM(time_to_first_token_ms), 0)",
    },
    MeasureSpec {
        name: "avg_time_to_first_token_ms",
        sql: "COALESCE(AVG(NULLIF(time_to_first_token_ms, 0)), 0)",
    },
    MeasureSpec {
        name: "success_count",
        sql: "COALESCE(SUM(success_count), 0)",
    },
    MeasureSpec {
        name: "error_count",
        sql: "COALESCE(SUM(error_count), 0)",
    },
];

const TOOL_MEASURES: &[MeasureSpec] = &[
    MeasureSpec {
        name: "call_count",
        sql: "COALESCE(SUM(call_count), 0)",
    },
    MeasureSpec {
        name: "success_count",
        sql: "COALESCE(SUM(success_count), 0)",
    },
    MeasureSpec {
        name: "error_count",
        sql: "COALESCE(SUM(error_count), 0)",
    },
    MeasureSpec {
        name: "timeout_count",
        sql: "COALESCE(SUM(timeout_count), 0)",
    },
    MeasureSpec {
        name: "cancelled_count",
        sql: "COALESCE(SUM(cancelled_count), 0)",
    },
    MeasureSpec {
        name: "duration_ms",
        sql: "COALESCE(SUM(duration_ms), 0)",
    },
    MeasureSpec {
        name: "avg_duration_ms",
        sql: "COALESCE(AVG(NULLIF(duration_ms, 0)), 0)",
    },
];

const TURN_MEASURES: &[MeasureSpec] = &[
    MeasureSpec {
        name: "turn_count",
        sql: "COALESCE(SUM(turn_count), 0)",
    },
    MeasureSpec {
        name: "duration_ms",
        sql: "COALESCE(SUM(duration_ms), 0)",
    },
    MeasureSpec {
        name: "avg_duration_ms",
        sql: "COALESCE(AVG(NULLIF(duration_ms, 0)), 0)",
    },
    MeasureSpec {
        name: "iterations",
        sql: "COALESCE(SUM(iterations), 0)",
    },
    MeasureSpec {
        name: "tool_call_count",
        sql: "COALESCE(SUM(tool_call_count), 0)",
    },
    MeasureSpec {
        name: "success_count",
        sql: "COALESCE(SUM(success_count), 0)",
    },
    MeasureSpec {
        name: "error_count",
        sql: "COALESCE(SUM(error_count), 0)",
    },
    MeasureSpec {
        name: "input_tokens",
        sql: "COALESCE(SUM(input_tokens), 0)",
    },
    MeasureSpec {
        name: "output_tokens",
        sql: "COALESCE(SUM(output_tokens), 0)",
    },
    MeasureSpec {
        name: "total_tokens",
        sql: "COALESCE(SUM(total_tokens), 0)",
    },
];

const SESSION_MEASURES: &[MeasureSpec] = &[
    MeasureSpec {
        name: "session_count",
        sql: "COALESCE(SUM(session_count), 0)",
    },
    MeasureSpec {
        name: "duration_ms",
        sql: "COALESCE(SUM(duration_ms), 0)",
    },
    MeasureSpec {
        name: "avg_duration_ms",
        sql: "COALESCE(AVG(NULLIF(duration_ms, 0)), 0)",
    },
    MeasureSpec {
        name: "turn_count",
        sql: "COALESCE(SUM(turn_count), 0)",
    },
    MeasureSpec {
        name: "tool_call_count",
        sql: "COALESCE(SUM(tool_call_count), 0)",
    },
    MeasureSpec {
        name: "input_tokens",
        sql: "COALESCE(SUM(input_tokens), 0)",
    },
    MeasureSpec {
        name: "output_tokens",
        sql: "COALESCE(SUM(output_tokens), 0)",
    },
    MeasureSpec {
        name: "total_tokens",
        sql: "COALESCE(SUM(total_tokens), 0)",
    },
];

const BUDGET_MEASURES: &[MeasureSpec] = &[
    MeasureSpec {
        name: "amount",
        sql: "COALESCE(SUM(amount), 0)",
    },
    MeasureSpec {
        name: "debit_count",
        sql: "COALESCE(SUM(debit_count), 0)",
    },
    MeasureSpec {
        name: "credit_count",
        sql: "COALESCE(SUM(credit_count), 0)",
    },
];

const DATASETS: &[DatasetSpec] = &[
    DatasetSpec {
        name: "llm_generations",
        table: "fact_llm_generation",
        dimensions: LLM_DIMS,
        measures: LLM_MEASURES,
        filters: LLM_DIMS,
    },
    DatasetSpec {
        name: "tool_calls",
        table: "fact_tool_call",
        dimensions: TOOL_DIMS,
        measures: TOOL_MEASURES,
        filters: TOOL_DIMS,
    },
    DatasetSpec {
        name: "turns",
        table: "fact_turn",
        dimensions: TURN_DIMS,
        measures: TURN_MEASURES,
        filters: TURN_DIMS,
    },
    DatasetSpec {
        name: "sessions",
        table: "fact_session",
        dimensions: SESSION_DIMS,
        measures: SESSION_MEASURES,
        filters: SESSION_DIMS,
    },
    DatasetSpec {
        name: "budget_postings",
        table: "fact_budget_posting",
        dimensions: BUDGET_DIMS,
        measures: BUDGET_MEASURES,
        filters: BUDGET_DIMS,
    },
];

pub fn catalog() -> DatasetCatalog {
    DatasetCatalog {
        datasets: DATASETS
            .iter()
            .map(|dataset| DatasetCatalogEntry {
                name: dataset.name.to_string(),
                dimensions: dataset
                    .dimensions
                    .iter()
                    .map(|field| field.name.to_string())
                    .collect(),
                measures: dataset
                    .measures
                    .iter()
                    .map(|measure| measure.name.to_string())
                    .collect(),
                filter_fields: dataset
                    .filters
                    .iter()
                    .map(|field| field.name.to_string())
                    .collect(),
            })
            .collect(),
    }
}

pub fn dataset(name: &str) -> Option<&'static DatasetSpec> {
    DATASETS.iter().find(|dataset| dataset.name == name)
}

pub fn dimension(dataset: &DatasetSpec, name: &str) -> Option<&'static FieldSpec> {
    dataset.dimensions.iter().find(|field| field.name == name)
}

pub fn filter_field(dataset: &DatasetSpec, name: &str) -> Option<&'static FieldSpec> {
    dataset.filters.iter().find(|field| field.name == name)
}

pub fn measure(dataset: &DatasetSpec, name: &str) -> Option<&'static MeasureSpec> {
    dataset.measures.iter().find(|measure| measure.name == name)
}

pub fn validate_query(query: &ReportQuery) -> Result<(), CommandError> {
    let dataset = dataset(&query.dataset)
        .ok_or_else(|| CommandError::bad_request("Unknown reporting dataset"))?;

    if query.time_range.from >= query.time_range.to {
        return Err(CommandError::bad_request(
            "time_range.from must be before time_range.to",
        ));
    }
    if query.time_range.to - query.time_range.from > chrono::Duration::days(MAX_RANGE_DAYS) {
        return Err(CommandError::bad_request(
            "Reporting time range is too large",
        ));
    }
    if query.limit == 0 || query.limit > MAX_REPORT_LIMIT {
        return Err(CommandError::bad_request(
            "Reporting limit must be between 1 and 1000",
        ));
    }
    if query.dimensions.len() > MAX_DIMENSIONS {
        return Err(CommandError::bad_request("Too many reporting dimensions"));
    }
    if query.measures.is_empty() || query.measures.len() > MAX_MEASURES {
        return Err(CommandError::bad_request(
            "Reporting query must include between 1 and 16 measures",
        ));
    }
    if query.filters.len() > MAX_FILTERS {
        return Err(CommandError::bad_request("Too many reporting filters"));
    }

    let mut seen_dimensions = BTreeMap::new();
    for name in &query.dimensions {
        if dimension(dataset, name).is_none() {
            return Err(CommandError::bad_request(
                "Invalid dimension for reporting dataset",
            ));
        }
        if seen_dimensions.insert(name, ()).is_some() {
            return Err(CommandError::bad_request("Duplicate reporting dimension"));
        }
    }

    let mut seen_measures = BTreeMap::new();
    for name in &query.measures {
        if measure(dataset, name).is_none() {
            return Err(CommandError::bad_request(
                "Invalid measure for reporting dataset",
            ));
        }
        if seen_measures.insert(name, ()).is_some() {
            return Err(CommandError::bad_request("Duplicate reporting measure"));
        }
    }

    for filter in &query.filters {
        if filter.field == "org_id" || filter_field(dataset, &filter.field).is_none() {
            return Err(CommandError::bad_request("Invalid reporting filter field"));
        }
        if matches!(filter.op, ReportFilterOp::In) && !filter.value.is_array() {
            return Err(CommandError::bad_request(
                "in filters require an array value",
            ));
        }
        if matches!(
            filter.op,
            ReportFilterOp::Gt | ReportFilterOp::Gte | ReportFilterOp::Lt | ReportFilterOp::Lte
        ) {
            return Err(CommandError::bad_request(
                "Reporting filters only support eq, neq, and in comparisons",
            ));
        }
    }

    for order in &query.order_by {
        validate_order_by(dataset, order)?;
    }
    Ok(())
}

fn validate_order_by(dataset: &DatasetSpec, order: &ReportOrderBy) -> Result<(), CommandError> {
    match (&order.dimension, &order.measure) {
        (Some(_), Some(_)) | (None, None) => Err(CommandError::bad_request(
            "Each order_by entry must specify exactly one dimension or measure",
        )),
        (Some(name), None) => {
            if dimension(dataset, name).is_none() {
                return Err(CommandError::bad_request("Invalid order_by dimension"));
            }
            Ok(())
        }
        (None, Some(name)) => {
            if measure(dataset, name).is_none() {
                return Err(CommandError::bad_request("Invalid order_by measure"));
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use everruns_core::reporting::{
        ReportFilter, ReportFilterOp, ReportOrderBy, ReportOrderDirection, ReportQuery,
        ReportTimeRange,
    };
    use serde_json::json;

    use super::{MAX_REPORT_LIMIT, catalog, validate_query};

    fn query() -> ReportQuery {
        let to = Utc::now();
        ReportQuery {
            dataset: "tool_calls".to_string(),
            time_range: ReportTimeRange {
                from: to - Duration::days(7),
                to,
            },
            dimensions: vec!["tool".to_string(), "day".to_string()],
            measures: vec!["call_count".to_string()],
            filters: vec![ReportFilter {
                field: "tool_status".to_string(),
                op: ReportFilterOp::Eq,
                value: json!("success"),
            }],
            order_by: vec![ReportOrderBy {
                dimension: None,
                measure: Some("call_count".to_string()),
                direction: ReportOrderDirection::Desc,
            }],
            limit: 100,
        }
    }

    #[test]
    fn catalog_exposes_phase_one_datasets() {
        let catalog = catalog();
        let names: Vec<_> = catalog
            .datasets
            .iter()
            .map(|dataset| dataset.name.as_str())
            .collect();
        assert!(names.contains(&"llm_generations"));
        assert!(names.contains(&"tool_calls"));
        assert!(names.contains(&"turns"));
        assert!(names.contains(&"sessions"));
        assert!(names.contains(&"budget_postings"));
    }

    #[test]
    fn validates_representative_tool_query() {
        validate_query(&query()).expect("valid report query");
    }

    #[test]
    fn rejects_org_id_filters() {
        let mut query = query();
        query.filters = vec![ReportFilter {
            field: "org_id".to_string(),
            op: ReportFilterOp::Eq,
            value: json!(1),
        }];

        assert!(validate_query(&query).is_err());
    }

    #[test]
    fn rejects_dimension_from_another_dataset() {
        let mut query = query();
        query.dimensions = vec!["model".to_string()];

        assert!(validate_query(&query).is_err());
    }

    #[test]
    fn enforces_bounded_limit_and_time_range() {
        let mut limit_query = query();
        limit_query.limit = MAX_REPORT_LIMIT + 1;
        assert!(validate_query(&limit_query).is_err());

        let mut range_query = query();
        range_query.time_range.from = range_query.time_range.to - Duration::days(371);
        assert!(validate_query(&range_query).is_err());
    }

    #[test]
    fn rejects_untyped_comparison_filters() {
        let mut query = query();
        query.filters = vec![ReportFilter {
            field: "tool".to_string(),
            op: ReportFilterOp::Gt,
            value: json!("bash"),
        }];

        assert!(validate_query(&query).is_err());
    }
}
