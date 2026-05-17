export type ReportColumnKind = "dimension" | "measure";
export type ReportFilterOp = "eq" | "neq" | "in";
export type ReportOrderDirection = "asc" | "desc";
export type ReportExportFormat = "csv" | "json";

export interface ReportTimeRange {
  from: string;
  to: string;
}

export interface ReportFilter {
  field: string;
  op: ReportFilterOp;
  value: unknown;
}

export interface ReportOrderBy {
  dimension?: string;
  measure?: string;
  direction?: ReportOrderDirection;
}

export interface ReportQuery {
  dataset: string;
  time_range: ReportTimeRange;
  dimensions: string[];
  measures: string[];
  filters?: ReportFilter[];
  order_by?: ReportOrderBy[];
  limit?: number;
}

export interface ReportColumn {
  name: string;
  kind: ReportColumnKind;
}

export interface ReportResult {
  as_of: string;
  freshness_lag_ms?: number | null;
  columns: ReportColumn[];
  rows: Record<string, unknown>[];
}

export interface DatasetCatalogEntry {
  name: string;
  dimensions: string[];
  measures: string[];
  filter_fields: string[];
}

export interface DatasetCatalog {
  datasets: DatasetCatalogEntry[];
}

export interface SavedReportDashboardMetadata {
  title?: string;
  section?: string;
  chart_type?: string;
  position?: number;
}

export interface SavedReport {
  id: string;
  name: string;
  description?: string;
  query: ReportQuery;
  dashboard?: SavedReportDashboardMetadata;
  created_at: string;
  updated_at: string;
}

export interface CreateSavedReportRequest {
  name: string;
  description?: string;
  query: ReportQuery;
  dashboard?: SavedReportDashboardMetadata;
}

export interface ReportExport {
  format: ReportExportFormat;
  filename: string;
  content_type: string;
  content: string;
  as_of: string;
  freshness_lag_ms?: number | null;
}

export interface DatasetProjectorLag {
  dataset: string;
  latest_projected_at?: string | null;
  freshness_lag_ms?: number | null;
}

export interface FailedReportingOutboxRow {
  id: string;
  org_id: number;
  source_type: string;
  source_id: string;
  attempts: number;
  last_error?: string | null;
  updated_at: string;
}

export interface ReportingOutboxDiagnostics {
  pending: number;
  processing: number;
  failed: number;
  completed: number;
  oldest_pending_at?: string | null;
  failed_rows: FailedReportingOutboxRow[];
}

export interface ReportingDiagnostics {
  generated_at: string;
  projector_lag: DatasetProjectorLag[];
  outbox: ReportingOutboxDiagnostics;
}

export interface ProjectorRunResult {
  claimed: number;
  completed: number;
  failed: number;
}

export interface ReportingBackfillResult {
  enqueued: number;
  events: number;
  sessions: number;
  llm_generations: number;
  usage_ledger: number;
}
