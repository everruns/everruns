use sqlx::{Acquire, PgPool};

/// Independent migration ledger used by the public Scale crate.
pub const MIGRATION_TABLE: &str = "everruns_scale_schema_migrations";

const MIGRATIONS: &[(i64, &str, &str)] = &[
    (1, "durable_substrate", DURABLE_SCHEMA),
    (2, "portable_agent_catalog", SCALE_SCHEMA),
    (3, "canonical_events_and_environments", SCALE_RUNTIME_SCHEMA),
];

/// Create or upgrade the Scale and generic durable schemas.
///
/// The ledger is intentionally separate from SQLx's product migration table,
/// so an external consumer can start from a blank PostgreSQL database and the
/// Everruns server can call this idempotently inside its larger schema.
pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut connection = pool.acquire().await?;
    sqlx::query("SELECT pg_advisory_lock(hashtext('everruns-scale-migrations'))")
        .execute(&mut *connection)
        .await?;
    let result = migrate_locked(&mut connection).await;
    let unlock = sqlx::query("SELECT pg_advisory_unlock(hashtext('everruns-scale-migrations'))")
        .execute(&mut *connection)
        .await;
    result?;
    unlock?;
    Ok(())
}

async fn migrate_locked(
    connection: &mut sqlx::pool::PoolConnection<sqlx::Postgres>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS everruns_scale_schema_migrations (\
         version BIGINT PRIMARY KEY, name TEXT NOT NULL, applied_at TIMESTAMPTZ NOT NULL DEFAULT now())",
    )
    .execute(&mut **connection)
    .await?;
    for (version, name, sql) in MIGRATIONS {
        let applied: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM everruns_scale_schema_migrations WHERE version = $1)",
        )
        .bind(version)
        .fetch_one(&mut **connection)
        .await?;
        if applied {
            continue;
        }
        let mut transaction = connection.begin().await?;
        sqlx::raw_sql(*sql).execute(&mut *transaction).await?;
        sqlx::query("INSERT INTO everruns_scale_schema_migrations (version, name) VALUES ($1, $2)")
            .bind(version)
            .bind(name)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
    }
    Ok(())
}

const SCALE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS everruns_scale_sessions (
    session_id UUID PRIMARY KEY,
    definition_version SMALLINT NOT NULL,
    portable_agent JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS everruns_scale_sessions_created_idx
    ON everruns_scale_sessions (created_at);
"#;

const SCALE_RUNTIME_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS everruns_scale_events (
    session_id UUID NOT NULL,
    sequence INT NOT NULL CHECK (sequence > 0),
    event_id UUID NOT NULL UNIQUE,
    envelope JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (session_id, sequence),
    FOREIGN KEY (session_id) REFERENCES everruns_scale_sessions(session_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS everruns_scale_environment_bindings (
    session_id UUID PRIMARY KEY,
    provider_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    head_id TEXT NOT NULL,
    isolated BOOLEAN NOT NULL,
    binding JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (session_id) REFERENCES everruns_scale_sessions(session_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS everruns_scale_environment_head_idx
    ON everruns_scale_environment_bindings (provider_id, workspace_id, head_id);
"#;

// Self-contained subset used by PostgresWorkflowEventStore and the neutral
// turn driver. Product migrations may add optional reporting/scheduler indexes.
const DURABLE_SCHEMA: &str = r#"
CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE IF NOT EXISTS durable_workflow_instances (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    input JSONB NOT NULL,
    result JSONB,
    error JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    partition_key INT NOT NULL DEFAULT 0,
    trace_id TEXT,
    span_id TEXT
);
CREATE INDEX IF NOT EXISTS idx_durable_workflow_instances_status ON durable_workflow_instances(status);

CREATE TABLE IF NOT EXISTS durable_workflow_events (
    id BIGSERIAL PRIMARY KEY,
    workflow_id UUID NOT NULL REFERENCES durable_workflow_instances(id) ON DELETE CASCADE,
    sequence_num INT NOT NULL,
    event_type TEXT NOT NULL,
    event_data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    trace_id TEXT,
    span_id TEXT,
    UNIQUE(workflow_id, sequence_num)
);
CREATE INDEX IF NOT EXISTS idx_durable_workflow_events_workflow
    ON durable_workflow_events(workflow_id, sequence_num);

CREATE TABLE IF NOT EXISTS durable_workflow_snapshots (
    id BIGSERIAL PRIMARY KEY,
    workflow_id UUID NOT NULL REFERENCES durable_workflow_instances(id) ON DELETE CASCADE,
    sequence_num INT NOT NULL,
    snapshot_data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(workflow_id, sequence_num)
);

CREATE TABLE IF NOT EXISTS durable_task_queue (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow_id UUID REFERENCES durable_workflow_instances(id) ON DELETE CASCADE,
    activity_id TEXT NOT NULL,
    activity_type TEXT NOT NULL,
    input JSONB NOT NULL,
    options JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    priority INT NOT NULL DEFAULT 0,
    scheduled_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    visible_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    claimed_by TEXT,
    claimed_at TIMESTAMPTZ,
    heartbeat_at TIMESTAMPTZ,
    attempt INT NOT NULL DEFAULT 0,
    max_attempts INT NOT NULL,
    last_error TEXT,
    schedule_to_start_timeout_ms BIGINT NOT NULL,
    start_to_close_timeout_ms BIGINT NOT NULL,
    heartbeat_timeout_ms BIGINT,
    trace_id TEXT,
    span_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    progress_token BIGINT,
    no_progress_count INT NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_durable_task_queue_pending
    ON durable_task_queue(priority DESC, visible_at, activity_type) WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS idx_durable_task_queue_claimed
    ON durable_task_queue(claimed_by, heartbeat_at) WHERE status = 'claimed';
CREATE INDEX IF NOT EXISTS idx_durable_task_queue_workflow ON durable_task_queue(workflow_id);

CREATE TABLE IF NOT EXISTS durable_dead_letter_queue (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    original_task_id UUID NOT NULL,
    workflow_id UUID REFERENCES durable_workflow_instances(id) ON DELETE CASCADE,
    activity_id TEXT NOT NULL,
    activity_type TEXT NOT NULL,
    input JSONB NOT NULL,
    attempts INT NOT NULL,
    last_error TEXT NOT NULL,
    error_history JSONB NOT NULL,
    dead_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    requeued_at TIMESTAMPTZ,
    requeue_count INT NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS durable_signals (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow_id UUID NOT NULL REFERENCES durable_workflow_instances(id) ON DELETE CASCADE,
    signal_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    sent_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    processed_at TIMESTAMPTZ,
    sequence_num SERIAL
);
CREATE INDEX IF NOT EXISTS idx_durable_signals_pending
    ON durable_signals(workflow_id, sequence_num) WHERE processed_at IS NULL;

CREATE TABLE IF NOT EXISTS durable_workers (
    id TEXT PRIMARY KEY,
    worker_group TEXT NOT NULL,
    activity_types TEXT[] NOT NULL,
    max_concurrency INT NOT NULL,
    current_load INT NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'active',
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    accepting_tasks BOOLEAN NOT NULL DEFAULT true,
    backpressure_reason TEXT,
    hostname TEXT,
    version TEXT,
    metadata JSONB
);

CREATE TABLE IF NOT EXISTS durable_circuit_breaker_state (
    key TEXT PRIMARY KEY,
    state TEXT NOT NULL DEFAULT 'closed',
    failure_count INT NOT NULL DEFAULT 0,
    success_count INT NOT NULL DEFAULT 0,
    last_failure_at TIMESTAMPTZ,
    opened_at TIMESTAMPTZ,
    half_open_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS durable_stat_counters (
    name TEXT PRIMARY KEY,
    value BIGINT NOT NULL DEFAULT 0 CHECK (value >= 0)
);
INSERT INTO durable_stat_counters(name, value) VALUES
 ('tasks_completed', 0), ('tasks_failed', 0), ('tasks_started', 0),
 ('workflows_completed', 0), ('workflows_failed', 0), ('workflows_started', 0)
ON CONFLICT (name) DO NOTHING;

CREATE OR REPLACE FUNCTION notify_durable_task_available() RETURNS TRIGGER AS $$
BEGIN
  PERFORM pg_notify('durable_task_available', NEW.activity_type);
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS durable_task_insert_notify ON durable_task_queue;
CREATE TRIGGER durable_task_insert_notify AFTER INSERT ON durable_task_queue
FOR EACH ROW EXECUTE FUNCTION notify_durable_task_available();
"#;
