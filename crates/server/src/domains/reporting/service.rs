use std::sync::Arc;

use everruns_core::reporting::{ReportQuery, ReportResult, ReportScope, ReportingQueryBackend};

use super::catalog;
use super::types::ProjectorRunResult;
use crate::domains::common::{CommandError, classify_anyhow};
use crate::storage::StorageBackend;
use crate::storage::reporting::{PostgresReportingProjector, PostgresReportingQueryBackend};

#[derive(Clone)]
pub struct ReportingService {
    db: Arc<StorageBackend>,
}

impl ReportingService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
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

    pub async fn run_projector_once(&self, limit: i64) -> Result<ProjectorRunResult, CommandError> {
        match self.db.as_ref() {
            StorageBackend::Postgres(db) => PostgresReportingProjector::new(db.pool().clone())
                .run_once(limit)
                .await
                .map_err(classify_anyhow),
            StorageBackend::InMemory(_) => Ok(ProjectorRunResult {
                claimed: 0,
                completed: 0,
                failed: 0,
            }),
        }
    }
}
