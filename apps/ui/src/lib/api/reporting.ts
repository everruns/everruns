import { api } from "./client";
import type {
  CreateSavedReportRequest,
  DatasetCatalog,
  ProjectorRunResult,
  ReportExport,
  ReportExportFormat,
  ReportQuery,
  ReportResult,
  ReportingBackfillResult,
  ReportingDiagnostics,
  SavedReport,
} from "./types";

interface ListResponse<T> {
  data: T[];
}

export async function getReportCatalog(): Promise<DatasetCatalog> {
  const response = await api.get<DatasetCatalog>("/v1/reports/catalog");
  return response.data;
}

export async function runReportQuery(query: ReportQuery): Promise<ReportResult> {
  const response = await api.post<ReportResult>("/v1/reports/query", query);
  return response.data;
}

export async function exportReportQuery(
  query: ReportQuery,
  format: ReportExportFormat,
): Promise<ReportExport> {
  const response = await api.post<ReportExport>("/v1/reports/query/export", { query, format });
  return response.data;
}

export async function listSavedReports(): Promise<SavedReport[]> {
  const response = await api.get<ListResponse<SavedReport>>("/v1/reports/saved");
  return response.data.data;
}

export async function createSavedReport(request: CreateSavedReportRequest): Promise<SavedReport> {
  const response = await api.post<SavedReport>("/v1/reports/saved", request);
  return response.data;
}

export async function deleteSavedReport(reportId: string): Promise<void> {
  await api.delete(`/v1/reports/saved/${reportId}`);
}

export async function runSavedReport(reportId: string): Promise<ReportResult> {
  const response = await api.post<ReportResult>(`/v1/reports/saved/${reportId}/run`, {});
  return response.data;
}

export async function exportSavedReport(
  reportId: string,
  format: ReportExportFormat,
): Promise<ReportExport> {
  const response = await api.post<ReportExport>(`/v1/reports/saved/${reportId}/export`, {
    format,
  });
  return response.data;
}

export async function getReportingDiagnostics(): Promise<ReportingDiagnostics> {
  const response = await api.get<ReportingDiagnostics>("/v1/reports/admin/diagnostics");
  return response.data;
}

export async function runReportingProjector(limit = 500): Promise<ProjectorRunResult> {
  const response = await api.post<ProjectorRunResult>(`/v1/reports/projector/run?limit=${limit}`);
  return response.data;
}

export async function backfillReporting(limit = 1000): Promise<ReportingBackfillResult> {
  const response = await api.post<ReportingBackfillResult>("/v1/reports/admin/backfill", {
    limit,
  });
  return response.data;
}
