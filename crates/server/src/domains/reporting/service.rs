use std::sync::Arc;

use chrono::{DateTime, Utc};
use everruns_core::reporting::{ReportQuery, ReportResult, ReportScope, ReportingQueryBackend};
use everruns_durable::UpdateField;
use parking_lot::RwLock;
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use super::catalog;
use super::types::{
    CreateSavedReportRequest, DatasetProjectorLag, FailedReportingOutboxRow, ProjectorRunResult,
    ReportExport, ReportExportFormat, ReportingBackfillResult, ReportingDiagnostics,
    ReportingOutboxDiagnostics, SavedReport, SavedReportDashboardMetadata,
    UpdateSavedReportRequest,
};
use crate::domains::common::{CommandError, classify_anyhow};
use crate::storage::StorageBackend;
use crate::storage::reporting::{PostgresReportingProjector, PostgresReportingQueryBackend};

const MAX_SAVED_REPORTS_LIST: usize = 1_000;

#[derive(Clone)]
pub struct ReportingService {
    db: Arc<StorageBackend>,
    memory_saved_reports: Arc<RwLock<Vec<SavedReportRecord>>>,
}

#[derive(Debug, Clone)]
struct SavedReportRecord {
    org_id: i64,
    report: SavedReport,
}

impl ReportingService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            db,
            memory_saved_reports: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub async fn query(
        &self,
        scope: ReportScope,
        query: ReportQuery,
    ) -> Result<ReportResult, CommandError> {
        catalog::validate_query(&query)?;
        match self.db.as_ref() {
            StorageBackend::Postgres(db) => PostgresReportingQueryBackend::new(db.pool().clone())
                .query(scope, query)
                .await
                .map_err(classify_anyhow),
            StorageBackend::InMemory(_) => Ok(ReportResult {
                as_of: chrono::Utc::now(),
                freshness_lag_ms: None,
                columns: query
                    .dimensions
                    .iter()
                    .map(|name| everruns_core::reporting::ReportColumn {
                        name: name.clone(),
                        kind: everruns_core::reporting::ReportColumnKind::Dimension,
                    })
                    .chain(query.measures.iter().map(|name| {
                        everruns_core::reporting::ReportColumn {
                            name: name.clone(),
                            kind: everruns_core::reporting::ReportColumnKind::Measure,
                        }
                    }))
                    .collect(),
                rows: Vec::new(),
            }),
        }
    }

    pub async fn list_saved_reports(&self, org_id: i64) -> Result<Vec<SavedReport>, CommandError> {
        match self.db.as_ref() {
            StorageBackend::Postgres(db) => {
                let rows = sqlx::query(
                    r#"
                    SELECT id, name, description, query, dashboard, created_at, updated_at
                      FROM reporting_saved_reports
                     WHERE org_id = $1
                  ORDER BY updated_at DESC, name ASC
                     LIMIT $2
                    "#,
                )
                .bind(org_id)
                .bind(MAX_SAVED_REPORTS_LIST as i64)
                .fetch_all(db.pool())
                .await
                .map_err(|err| classify_anyhow(err.into()))?;
                rows.into_iter().map(saved_report_from_row).collect()
            }
            StorageBackend::InMemory(_) => Ok(self
                .memory_saved_reports
                .read()
                .iter()
                .filter(|row| row.org_id == org_id)
                .take(MAX_SAVED_REPORTS_LIST)
                .map(|row| row.report.clone())
                .collect()),
        }
    }

    pub async fn get_saved_report(
        &self,
        org_id: i64,
        report_id: Uuid,
    ) -> Result<SavedReport, CommandError> {
        match self.db.as_ref() {
            StorageBackend::Postgres(db) => {
                let row = sqlx::query(
                    r#"
                    SELECT id, name, description, query, dashboard, created_at, updated_at
                      FROM reporting_saved_reports
                     WHERE org_id = $1 AND id = $2
                    "#,
                )
                .bind(org_id)
                .bind(report_id)
                .fetch_optional(db.pool())
                .await
                .map_err(|err| classify_anyhow(err.into()))?
                .ok_or_else(|| CommandError::not_found("Saved report"))?;
                saved_report_from_row(row)
            }
            StorageBackend::InMemory(_) => self
                .memory_saved_reports
                .read()
                .iter()
                .find(|row| row.org_id == org_id && row.report.id == report_id)
                .map(|row| row.report.clone())
                .ok_or_else(|| CommandError::not_found("Saved report")),
        }
    }

    pub async fn create_saved_report(
        &self,
        org_id: i64,
        request: CreateSavedReportRequest,
    ) -> Result<SavedReport, CommandError> {
        validate_saved_report_request(
            &request.name,
            request.description.as_deref(),
            request.dashboard.as_ref(),
        )?;
        catalog::validate_query(&request.query)?;

        match self.db.as_ref() {
            StorageBackend::Postgres(db) => {
                let row = sqlx::query(
                    r#"
                    INSERT INTO reporting_saved_reports (org_id, name, description, query, dashboard)
                    VALUES ($1, $2, $3, $4, $5)
                    RETURNING id, name, description, query, dashboard, created_at, updated_at
                    "#,
                )
                .bind(org_id)
                .bind(request.name)
                .bind(request.description)
                .bind(serde_json::to_value(request.query).map_err(|err| CommandError::internal(err.into()))?)
                .bind(
                    request
                        .dashboard
                        .map(serde_json::to_value)
                        .transpose()
                        .map_err(|err| CommandError::internal(err.into()))?,
                )
                .fetch_one(db.pool())
                .await
                .map_err(|err| classify_anyhow(err.into()))?;
                saved_report_from_row(row)
            }
            StorageBackend::InMemory(_) => {
                let now = Utc::now();
                let report = SavedReport {
                    id: Uuid::now_v7(),
                    name: request.name,
                    description: request.description,
                    query: request.query,
                    dashboard: request.dashboard,
                    created_at: now,
                    updated_at: now,
                };
                self.memory_saved_reports.write().push(SavedReportRecord {
                    org_id,
                    report: report.clone(),
                });
                Ok(report)
            }
        }
    }

    pub async fn update_saved_report(
        &self,
        org_id: i64,
        report_id: Uuid,
        request: UpdateSavedReportRequest,
    ) -> Result<SavedReport, CommandError> {
        validate_query_update(&request.query)?;

        match self.db.as_ref() {
            StorageBackend::Postgres(db) => {
                let current = self.get_saved_report(org_id, report_id).await?;
                let name = request.name.unwrap_or(current.name);
                let description = apply_update_field(current.description, request.description);
                let query = apply_query_update(current.query, request.query)?;
                let dashboard = apply_update_field(current.dashboard, request.dashboard);
                validate_saved_report_request(&name, description.as_deref(), dashboard.as_ref())?;

                let row = sqlx::query(
                    r#"
                    UPDATE reporting_saved_reports
                       SET name = $3,
                           description = $4,
                           query = $5,
                           dashboard = $6,
                           updated_at = NOW()
                     WHERE org_id = $1 AND id = $2
                 RETURNING id, name, description, query, dashboard, created_at, updated_at
                    "#,
                )
                .bind(org_id)
                .bind(report_id)
                .bind(name)
                .bind(description)
                .bind(
                    serde_json::to_value(query)
                        .map_err(|err| CommandError::internal(err.into()))?,
                )
                .bind(
                    dashboard
                        .map(serde_json::to_value)
                        .transpose()
                        .map_err(|err| CommandError::internal(err.into()))?,
                )
                .fetch_optional(db.pool())
                .await
                .map_err(|err| classify_anyhow(err.into()))?
                .ok_or_else(|| CommandError::not_found("Saved report"))?;
                saved_report_from_row(row)
            }
            StorageBackend::InMemory(_) => {
                let mut rows = self.memory_saved_reports.write();
                let row = rows
                    .iter_mut()
                    .find(|row| row.org_id == org_id && row.report.id == report_id)
                    .ok_or_else(|| CommandError::not_found("Saved report"))?;
                let name = request.name.unwrap_or_else(|| row.report.name.clone());
                let description =
                    apply_update_field(row.report.description.clone(), request.description);
                let query = apply_query_update(row.report.query.clone(), request.query)?;
                let dashboard = apply_update_field(row.report.dashboard.clone(), request.dashboard);
                validate_saved_report_request(&name, description.as_deref(), dashboard.as_ref())?;
                row.report.name = name;
                row.report.description = description;
                row.report.query = query;
                row.report.dashboard = dashboard;
                row.report.updated_at = Utc::now();
                Ok(row.report.clone())
            }
        }
    }

    pub async fn delete_saved_report(
        &self,
        org_id: i64,
        report_id: Uuid,
    ) -> Result<(), CommandError> {
        match self.db.as_ref() {
            StorageBackend::Postgres(db) => {
                let result = sqlx::query(
                    "DELETE FROM reporting_saved_reports WHERE org_id = $1 AND id = $2",
                )
                .bind(org_id)
                .bind(report_id)
                .execute(db.pool())
                .await
                .map_err(|err| classify_anyhow(err.into()))?;
                if result.rows_affected() == 0 {
                    return Err(CommandError::not_found("Saved report"));
                }
                Ok(())
            }
            StorageBackend::InMemory(_) => {
                let mut rows = self.memory_saved_reports.write();
                let before = rows.len();
                rows.retain(|row| !(row.org_id == org_id && row.report.id == report_id));
                if rows.len() == before {
                    return Err(CommandError::not_found("Saved report"));
                }
                Ok(())
            }
        }
    }

    pub async fn run_saved_report(
        &self,
        scope: ReportScope,
        report_id: Uuid,
    ) -> Result<ReportResult, CommandError> {
        let report = self.get_saved_report(scope.org_id, report_id).await?;
        self.query(scope, report.query).await
    }

    pub async fn export_query(
        &self,
        scope: ReportScope,
        query: ReportQuery,
        format: ReportExportFormat,
        filename_stem: &str,
    ) -> Result<ReportExport, CommandError> {
        let result = self.query(scope, query).await?;
        Ok(export_result(result, format, filename_stem)?)
    }

    pub async fn export_saved_report(
        &self,
        scope: ReportScope,
        report_id: Uuid,
        format: ReportExportFormat,
    ) -> Result<ReportExport, CommandError> {
        let report = self.get_saved_report(scope.org_id, report_id).await?;
        let filename = slugify_filename(&report.name);
        self.export_query(scope, report.query, format, &filename)
            .await
    }

    pub async fn diagnostics(&self, org_id: i64) -> Result<ReportingDiagnostics, CommandError> {
        match self.db.as_ref() {
            StorageBackend::Postgres(db) => {
                let generated_at = Utc::now();
                let outbox = reporting_outbox_diagnostics(db.pool(), org_id).await?;
                let mut projector_lag = Vec::new();
                for dataset in catalog::datasets() {
                    projector_lag
                        .push(dataset_lag(db.pool(), dataset.name, dataset.table, org_id).await?);
                }
                Ok(ReportingDiagnostics {
                    generated_at,
                    projector_lag,
                    outbox,
                })
            }
            StorageBackend::InMemory(_) => Ok(ReportingDiagnostics {
                generated_at: Utc::now(),
                projector_lag: catalog::datasets()
                    .iter()
                    .map(|dataset| DatasetProjectorLag {
                        dataset: dataset.name.to_string(),
                        latest_projected_at: None,
                        freshness_lag_ms: None,
                    })
                    .collect(),
                outbox: ReportingOutboxDiagnostics {
                    pending: 0,
                    processing: 0,
                    failed: 0,
                    completed: 0,
                    oldest_pending_at: None,
                    failed_rows: Vec::new(),
                },
            }),
        }
    }

    pub async fn run_projector_once(
        &self,
        org_id: i64,
        limit: i64,
    ) -> Result<ProjectorRunResult, CommandError> {
        match self.db.as_ref() {
            StorageBackend::Postgres(db) => PostgresReportingProjector::new(db.pool().clone())
                .run_once(org_id, limit)
                .await
                .map_err(classify_anyhow),
            StorageBackend::InMemory(_) => Ok(ProjectorRunResult {
                claimed: 0,
                completed: 0,
                failed: 0,
            }),
        }
    }

    pub async fn run_projector_due(&self, limit: i64) -> Result<ProjectorRunResult, CommandError> {
        match self.db.as_ref() {
            StorageBackend::Postgres(db) => PostgresReportingProjector::new(db.pool().clone())
                .run_due(limit)
                .await
                .map_err(classify_anyhow),
            StorageBackend::InMemory(_) => Ok(ProjectorRunResult {
                claimed: 0,
                completed: 0,
                failed: 0,
            }),
        }
    }

    pub async fn repair_missing_event_projections(&self, limit: i64) -> Result<i64, CommandError> {
        match self.db.as_ref() {
            StorageBackend::Postgres(db) => PostgresReportingProjector::new(db.pool().clone())
                .repair_missing_event_projections(limit)
                .await
                .map_err(classify_anyhow),
            StorageBackend::InMemory(_) => Ok(0),
        }
    }

    pub async fn backfill_missing(
        &self,
        org_id: Option<i64>,
        limit: i64,
    ) -> Result<ReportingBackfillResult, CommandError> {
        match self.db.as_ref() {
            StorageBackend::Postgres(db) => PostgresReportingProjector::new(db.pool().clone())
                .backfill_missing(org_id, limit)
                .await
                .map_err(classify_anyhow),
            StorageBackend::InMemory(_) => Ok(ReportingBackfillResult {
                enqueued: 0,
                events: 0,
                sessions: 0,
                llm_generations: 0,
                usage_ledger: 0,
            }),
        }
    }
}

fn saved_report_from_row(row: sqlx::postgres::PgRow) -> Result<SavedReport, CommandError> {
    let query: Value = row
        .try_get("query")
        .map_err(|err| classify_anyhow(err.into()))?;
    let dashboard: Option<Value> = row
        .try_get("dashboard")
        .map_err(|err| classify_anyhow(err.into()))?;
    Ok(SavedReport {
        id: row
            .try_get("id")
            .map_err(|err| classify_anyhow(err.into()))?,
        name: row
            .try_get("name")
            .map_err(|err| classify_anyhow(err.into()))?,
        description: row
            .try_get("description")
            .map_err(|err| classify_anyhow(err.into()))?,
        query: serde_json::from_value(query).map_err(|err| CommandError::internal(err.into()))?,
        dashboard: dashboard
            .map(serde_json::from_value)
            .transpose()
            .map_err(|err| CommandError::internal(err.into()))?,
        created_at: row
            .try_get("created_at")
            .map_err(|err| classify_anyhow(err.into()))?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|err| classify_anyhow(err.into()))?,
    })
}

fn apply_update_field<T>(current: Option<T>, field: UpdateField<T>) -> Option<T> {
    match field {
        UpdateField::Unchanged => current,
        UpdateField::Clear => None,
        UpdateField::Set(value) => Some(value),
    }
}

fn validate_query_update(field: &UpdateField<ReportQuery>) -> Result<(), CommandError> {
    match field {
        UpdateField::Unchanged => Ok(()),
        UpdateField::Clear => Err(CommandError::bad_request(
            "saved report query cannot be null",
        )),
        UpdateField::Set(query) => catalog::validate_query(query),
    }
}

fn apply_query_update(
    current: ReportQuery,
    field: UpdateField<ReportQuery>,
) -> Result<ReportQuery, CommandError> {
    match field {
        UpdateField::Unchanged => Ok(current),
        UpdateField::Clear => Err(CommandError::bad_request(
            "saved report query cannot be null",
        )),
        UpdateField::Set(query) => Ok(query),
    }
}

fn validate_saved_report_request(
    name: &str,
    description: Option<&str>,
    dashboard: Option<&SavedReportDashboardMetadata>,
) -> Result<(), CommandError> {
    if name.trim().is_empty() || name.chars().count() > 200 {
        return Err(CommandError::bad_request(
            "Saved report name must be between 1 and 200 characters",
        ));
    }
    if description.is_some_and(|value| value.chars().count() > 2_000) {
        return Err(CommandError::bad_request(
            "Saved report description must be at most 2000 characters",
        ));
    }
    if let Some(dashboard) = dashboard {
        validate_optional_label(dashboard.title.as_deref(), "Dashboard title", 200)?;
        validate_optional_label(dashboard.section.as_deref(), "Dashboard section", 100)?;
        validate_optional_label(dashboard.chart_type.as_deref(), "Dashboard chart_type", 64)?;
    }
    Ok(())
}

fn validate_optional_label(
    value: Option<&str>,
    label: &str,
    max_chars: usize,
) -> Result<(), CommandError> {
    if value.is_some_and(|value| value.chars().count() > max_chars) {
        return Err(CommandError::bad_request(format!(
            "{label} must be at most {max_chars} characters"
        )));
    }
    Ok(())
}

fn export_result(
    result: ReportResult,
    format: ReportExportFormat,
    filename_stem: &str,
) -> anyhow::Result<ReportExport> {
    let (content_type, extension, content) = match format {
        ReportExportFormat::Csv => ("text/csv; charset=utf-8", "csv", result_to_csv(&result)),
        ReportExportFormat::Json => (
            "application/json",
            "json",
            serde_json::to_string_pretty(&result)?,
        ),
    };
    Ok(ReportExport {
        format,
        filename: format!("{filename_stem}.{extension}"),
        content_type: content_type.to_string(),
        content,
        as_of: result.as_of,
        freshness_lag_ms: result.freshness_lag_ms,
    })
}

fn result_to_csv(result: &ReportResult) -> String {
    let mut out = String::new();
    out.push_str(
        &result
            .columns
            .iter()
            .map(|column| escape_csv_cell(&column.name))
            .collect::<Vec<_>>()
            .join(","),
    );
    out.push('\n');

    for row in &result.rows {
        let object = row.as_object();
        let cells = result
            .columns
            .iter()
            .map(|column| {
                let value = object.and_then(|object| object.get(&column.name));
                escape_csv_cell(&csv_value(value))
            })
            .collect::<Vec<_>>()
            .join(",");
        out.push_str(&cells);
        out.push('\n');
    }
    out
}

fn csv_value(value: Option<&Value>) -> String {
    match value {
        Some(Value::Null) | None => String::new(),
        Some(Value::String(value)) => value.clone(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        Some(value) => value.to_string(),
    }
}

fn escape_csv_cell(value: &str) -> String {
    // THREAT[TM-API-019]: Prevent spreadsheet formula execution when exported CSVs are
    // opened by prefixing formula-like cells before RFC 4180 escaping.
    let mut value = if matches!(
        value.as_bytes().first(),
        Some(b'=' | b'+' | b'-' | b'@' | b'\t' | b'\r' | b'\n')
    ) {
        format!("'{value}")
    } else {
        value.to_string()
    };
    if value.contains('"') {
        value = value.replace('"', "\"\"");
    }
    format!("\"{value}\"")
}

fn slugify_filename(value: &str) -> String {
    let slug = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "report".to_string()
    } else {
        slug
    }
}

async fn reporting_outbox_diagnostics(
    pool: &sqlx::PgPool,
    org_id: i64,
) -> Result<ReportingOutboxDiagnostics, CommandError> {
    let counts = sqlx::query(
        r#"
        SELECT
            COUNT(*) FILTER (WHERE status = 'pending') AS pending,
            COUNT(*) FILTER (WHERE status = 'processing') AS processing,
            COUNT(*) FILTER (WHERE status = 'failed') AS failed,
            COUNT(*) FILTER (WHERE status = 'completed') AS completed,
            MIN(created_at) FILTER (WHERE status = 'pending') AS oldest_pending_at
          FROM reporting_outbox
         WHERE org_id = $1
        "#,
    )
    .bind(org_id)
    .fetch_one(pool)
    .await
    .map_err(|err| classify_anyhow(err.into()))?;

    let failed_rows = sqlx::query(
        r#"
        SELECT id, org_id, source_type, source_id, attempts, last_error, updated_at
          FROM reporting_outbox
         WHERE org_id = $1 AND status = 'failed'
      ORDER BY updated_at DESC
         LIMIT 50
        "#,
    )
    .bind(org_id)
    .fetch_all(pool)
    .await
    .map_err(|err| classify_anyhow(err.into()))?
    .into_iter()
    .map(|row| {
        Ok(FailedReportingOutboxRow {
            id: row
                .try_get("id")
                .map_err(|err| classify_anyhow(err.into()))?,
            org_id: row
                .try_get("org_id")
                .map_err(|err| classify_anyhow(err.into()))?,
            source_type: row
                .try_get("source_type")
                .map_err(|err| classify_anyhow(err.into()))?,
            source_id: row
                .try_get("source_id")
                .map_err(|err| classify_anyhow(err.into()))?,
            attempts: row
                .try_get("attempts")
                .map_err(|err| classify_anyhow(err.into()))?,
            last_error: row
                .try_get("last_error")
                .map_err(|err| classify_anyhow(err.into()))?,
            updated_at: row
                .try_get("updated_at")
                .map_err(|err| classify_anyhow(err.into()))?,
        })
    })
    .collect::<Result<Vec<_>, CommandError>>()?;

    Ok(ReportingOutboxDiagnostics {
        pending: counts
            .try_get::<Option<i64>, _>("pending")
            .map_err(|err| classify_anyhow(err.into()))?
            .unwrap_or_default(),
        processing: counts
            .try_get::<Option<i64>, _>("processing")
            .map_err(|err| classify_anyhow(err.into()))?
            .unwrap_or_default(),
        failed: counts
            .try_get::<Option<i64>, _>("failed")
            .map_err(|err| classify_anyhow(err.into()))?
            .unwrap_or_default(),
        completed: counts
            .try_get::<Option<i64>, _>("completed")
            .map_err(|err| classify_anyhow(err.into()))?
            .unwrap_or_default(),
        oldest_pending_at: counts
            .try_get("oldest_pending_at")
            .map_err(|err| classify_anyhow(err.into()))?,
        failed_rows,
    })
}

async fn dataset_lag(
    pool: &sqlx::PgPool,
    dataset_name: &str,
    table: &str,
    org_id: i64,
) -> Result<DatasetProjectorLag, CommandError> {
    let sql =
        format!("SELECT MAX(projected_at) AS latest_projected_at FROM {table} WHERE org_id = $1");
    let row = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(org_id)
        .fetch_one(pool)
        .await
        .map_err(|err| classify_anyhow(err.into()))?;
    let latest_projected_at: Option<DateTime<Utc>> = row
        .try_get("latest_projected_at")
        .map_err(|err| classify_anyhow(err.into()))?;
    Ok(DatasetProjectorLag {
        dataset: dataset_name.to_string(),
        latest_projected_at,
        freshness_lag_ms: latest_projected_at
            .map(|latest| (Utc::now() - latest).num_milliseconds().max(0)),
    })
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use everruns_core::reporting::{ReportColumn, ReportColumnKind, ReportResult};
    use serde_json::json;

    use super::{escape_csv_cell, result_to_csv};

    #[test]
    fn escape_csv_cell_prefixes_formula_like_leading_chars() {
        assert_eq!(escape_csv_cell("=1+1"), "\"'=1+1\"");
        assert_eq!(escape_csv_cell("\t=1+1"), "\"'\t=1+1\"");
        assert_eq!(escape_csv_cell("\r=1+1"), "\"'\r=1+1\"");
        assert_eq!(escape_csv_cell("\n=1+1"), "\"'\n=1+1\"");
    }

    #[test]
    fn result_to_csv_escapes_lf_prefixed_formula_cells() {
        let result = ReportResult {
            as_of: Utc::now(),
            freshness_lag_ms: None,
            columns: vec![ReportColumn {
                name: "name".to_string(),
                kind: ReportColumnKind::Dimension,
            }],
            rows: vec![json!({ "name": "\n=1+1" })],
        };

        assert_eq!(result_to_csv(&result), "\"name\"\n\"'\n=1+1\"\n");
    }
}
