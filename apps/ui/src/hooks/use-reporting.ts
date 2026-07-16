"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";
import {
  backfillReporting,
  createSavedReport,
  deleteSavedReport,
  exportReportQuery,
  exportSavedReport,
  getReportCatalog,
  getReportingDiagnostics,
  listSavedReports,
  runReportQuery,
  runReportingProjector,
  runSavedReport,
} from "@/lib/api/reporting";
import type { CreateSavedReportRequest, ReportExportFormat, ReportQuery } from "@/lib/api/types";

export function useReportCatalog() {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useQuery({
    queryKey: queryKeys.reporting.catalog(org),
    queryFn: getReportCatalog,
    enabled: !!org,
  });
}

export function useReportQuery(query: ReportQuery | undefined) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useQuery({
    queryKey: queryKeys.reporting.query(org, query as unknown as Record<string, unknown>),
    queryFn: () => runReportQuery(query!),
    enabled: !!org && !!query,
  });
}

export function useSavedReports(enabled = true) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useQuery({
    queryKey: queryKeys.reporting.saved(org),
    queryFn: listSavedReports,
    enabled: !!org && enabled,
  });
}

export function useCreateSavedReport() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (request: CreateSavedReportRequest) => createSavedReport(request),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.reporting.saved(org) });
    },
  });
}

export function useDeleteSavedReport() {
  const queryClient = useQueryClient();
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useMutation({
    mutationFn: (reportId: string) => deleteSavedReport(reportId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.reporting.saved(org) });
    },
  });
}

export function useRunSavedReport() {
  return useMutation({
    mutationFn: (reportId: string) => runSavedReport(reportId),
  });
}

export function useExportReportQuery() {
  return useMutation({
    mutationFn: ({ query, format }: { query: ReportQuery; format: ReportExportFormat }) =>
      exportReportQuery(query, format),
  });
}

export function useExportSavedReport() {
  return useMutation({
    mutationFn: ({ reportId, format }: { reportId: string; format: ReportExportFormat }) =>
      exportSavedReport(reportId, format),
  });
}

export function useReportingDiagnostics(enabled = true) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  return useQuery({
    queryKey: queryKeys.reporting.diagnostics(org),
    queryFn: getReportingDiagnostics,
    enabled: enabled && !!org,
    refetchInterval: 30_000,
    retry: false,
  });
}

export function useRunReportingProjector() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: () => runReportingProjector(),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.reporting.all });
    },
  });
}

export function useBackfillReporting() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: () => backfillReporting(),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.reporting.all });
    },
  });
}
