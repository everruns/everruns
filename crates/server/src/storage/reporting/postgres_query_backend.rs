use anyhow::{Context, Result};
use async_trait::async_trait;
use everruns_core::reporting::{
    ReportColumn, ReportColumnKind, ReportFilterOp, ReportOrderDirection, ReportQuery,
    ReportResult, ReportScope, ReportingQueryBackend,
};
use serde_json::{Map, Value};
use sqlx::{PgPool, QueryBuilder, Row};

use crate::domains::reporting::catalog;

#[derive(Clone)]
pub struct PostgresReportingQueryBackend {
    pool: PgPool,
}

impl PostgresReportingQueryBackend {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ReportingQueryBackend for PostgresReportingQueryBackend {
    async fn query(&self, scope: ReportScope, query: ReportQuery) -> Result<ReportResult> {
        let dataset =
            catalog::dataset(&query.dataset).context("reporting dataset already validated")?;
        let mut columns = Vec::new();
        let mut select_parts = Vec::new();
        let mut group_parts = Vec::new();

        for dimension_name in &query.dimensions {
            let dimension = catalog::dimension(dataset, dimension_name)
                .context("dimension already validated")?;
            select_parts.push(format!(
                "to_jsonb({}) AS \"{}\"",
                dimension.sql, dimension.name
            ));
            group_parts.push(dimension.sql.to_string());
            columns.push(ReportColumn {
                name: dimension.name.to_string(),
                kind: ReportColumnKind::Dimension,
            });
        }

        for measure_name in &query.measures {
            let measure =
                catalog::measure(dataset, measure_name).context("measure already validated")?;
            select_parts.push(format!("to_jsonb({}) AS \"{}\"", measure.sql, measure.name));
            columns.push(ReportColumn {
                name: measure.name.to_string(),
                kind: ReportColumnKind::Measure,
            });
        }

        let mut builder = QueryBuilder::new(format!(
            "SELECT {} FROM {} WHERE org_id = ",
            select_parts.join(", "),
            dataset.table
        ));
        builder.push_bind(scope.org_id);
        builder.push(" AND created_at >= ");
        builder.push_bind(query.time_range.from);
        builder.push(" AND created_at <= ");
        builder.push_bind(query.time_range.to);

        for filter in &query.filters {
            let field = catalog::filter_field(dataset, &filter.field)
                .context("filter already validated")?;
            builder.push(" AND ");
            builder.push(field.sql);
            match filter.op {
                ReportFilterOp::Eq => {
                    builder.push("::TEXT = ");
                    bind_scalar_as_text(&mut builder, &filter.value)?;
                }
                ReportFilterOp::Neq => {
                    builder.push("::TEXT <> ");
                    bind_scalar_as_text(&mut builder, &filter.value)?;
                }
                ReportFilterOp::In => {
                    builder.push("::TEXT = ANY(");
                    builder.push_bind(string_array(&filter.value)?);
                    builder.push(")");
                }
                ReportFilterOp::Gt => {
                    builder.push(" > ");
                    bind_comparable(&mut builder, &filter.value)?;
                }
                ReportFilterOp::Gte => {
                    builder.push(" >= ");
                    bind_comparable(&mut builder, &filter.value)?;
                }
                ReportFilterOp::Lt => {
                    builder.push(" < ");
                    bind_comparable(&mut builder, &filter.value)?;
                }
                ReportFilterOp::Lte => {
                    builder.push(" <= ");
                    bind_comparable(&mut builder, &filter.value)?;
                }
            }
        }

        if !group_parts.is_empty() {
            builder.push(" GROUP BY ");
            builder.push(group_parts.join(", "));
        }

        if !query.order_by.is_empty() {
            builder.push(" ORDER BY ");
            for (index, order) in query.order_by.iter().enumerate() {
                if index > 0 {
                    builder.push(", ");
                }
                let name = order
                    .dimension
                    .as_deref()
                    .or(order.measure.as_deref())
                    .context("order_by already validated")?;
                builder.push("\"");
                builder.push(name);
                builder.push("\" ");
                builder.push(match order.direction {
                    ReportOrderDirection::Asc => "ASC",
                    ReportOrderDirection::Desc => "DESC",
                });
            }
        }

        builder.push(" LIMIT ");
        builder.push_bind(i64::from(query.limit));

        let rows = builder.build().fetch_all(&self.pool).await?;
        let mut result_rows = Vec::with_capacity(rows.len());
        for row in rows {
            let mut object = Map::new();
            for column in &columns {
                let value: Value = row.try_get(column.name.as_str())?;
                object.insert(column.name.clone(), value);
            }
            result_rows.push(Value::Object(object));
        }

        let freshness_lag_ms = latest_projection_lag_ms(&self.pool, dataset.table, scope.org_id)
            .await
            .ok()
            .flatten();

        Ok(ReportResult {
            as_of: chrono::Utc::now(),
            freshness_lag_ms,
            columns,
            rows: result_rows,
        })
    }
}

fn bind_scalar_as_text(builder: &mut QueryBuilder<sqlx::Postgres>, value: &Value) -> Result<()> {
    let value = match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        _ => anyhow::bail!("reporting filter value must be a scalar"),
    };
    builder.push_bind(value);
    Ok(())
}

fn bind_comparable(builder: &mut QueryBuilder<sqlx::Postgres>, value: &Value) -> Result<()> {
    match value {
        Value::Number(number) => {
            builder.push_bind(number.as_f64().context("invalid numeric filter value")?);
        }
        Value::String(value) => {
            builder.push_bind(value.clone());
        }
        _ => anyhow::bail!("comparison reporting filter value must be string or number"),
    }
    Ok(())
}

fn string_array(value: &Value) -> Result<Vec<String>> {
    let array = value
        .as_array()
        .context("reporting in filter value must be an array")?;
    array
        .iter()
        .map(|value| match value {
            Value::String(value) => Ok(value.clone()),
            Value::Number(value) => Ok(value.to_string()),
            Value::Bool(value) => Ok(value.to_string()),
            _ => anyhow::bail!("reporting in filter array values must be scalars"),
        })
        .collect()
}

async fn latest_projection_lag_ms(pool: &PgPool, table: &str, org_id: i64) -> Result<Option<i64>> {
    let sql = format!(
        "SELECT EXTRACT(EPOCH FROM (NOW() - MAX(projected_at))) * 1000 FROM {table} WHERE org_id = $1"
    );
    let row: Option<(Option<f64>,)> = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(org_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|row| row.0).map(|lag| lag.max(0.0) as i64))
}
