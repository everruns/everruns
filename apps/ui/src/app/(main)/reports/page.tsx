"use client";

import { useEffect, useMemo, useState } from "react";
import {
  BarChart3,
  Database,
  Download,
  Play,
  RefreshCw,
  RotateCw,
  Save,
  Trash2,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { PageBody, PageHeader, PageShell } from "@/components/layout";
import {
  useBackfillReporting,
  useCreateSavedReport,
  useDeleteSavedReport,
  useExportReportQuery,
  useExportSavedReport,
  usePageTitle,
  useReportCatalog,
  useReportQuery,
  useReportingDiagnostics,
  useRunReportingProjector,
  useRunSavedReport,
  useSavedReports,
} from "@/hooks";
import { useOrg } from "@/providers/org-provider";
import type {
  DatasetCatalogEntry,
  ReportExport,
  ReportQuery,
  ReportResult,
  SavedReport,
} from "@/lib/api/types";

const DATASET_LABELS: Record<string, string> = {
  llm_generations: "LLM Generations",
  tool_calls: "Tool Calls",
  turns: "Turns",
  sessions: "Sessions",
  budget_postings: "Budget Postings",
  capability_usage: "Capability Usage",
};

const RANGE_OPTIONS = [
  { value: "1", label: "24h" },
  { value: "7", label: "7d" },
  { value: "30", label: "30d" },
  { value: "90", label: "90d" },
];

function range(days: number) {
  const to = new Date();
  const from = new Date(to.getTime() - days * 24 * 60 * 60 * 1000);
  return {
    from: from.toISOString(),
    to: to.toISOString(),
  };
}

function formatDatasetName(name: string) {
  return DATASET_LABELS[name] ?? name.replaceAll("_", " ");
}

function formatValue(value: unknown) {
  if (value === null || value === undefined || value === "") return "-";
  if (typeof value === "number") {
    return Number.isInteger(value) ? value.toLocaleString() : value.toFixed(2);
  }
  if (typeof value === "string" && /^\d{4}-\d{2}-\d{2}T/.test(value)) {
    return new Date(value).toLocaleString();
  }
  return String(value);
}

function freshnessLabel(ms?: number | null) {
  if (ms === undefined || ms === null) return "No projections";
  if (ms < 60_000) return `${Math.round(ms / 1000)}s`;
  if (ms < 3_600_000) return `${Math.round(ms / 60_000)}m`;
  return `${Math.round(ms / 3_600_000)}h`;
}

function saveExport(file: ReportExport) {
  const blob = new Blob([file.content], { type: file.content_type });
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = file.filename;
  anchor.click();
  URL.revokeObjectURL(url);
}

function defaultQuery(dataset: DatasetCatalogEntry | undefined, days = 7): ReportQuery | undefined {
  if (!dataset) return undefined;
  return {
    dataset: dataset.name,
    time_range: range(days),
    dimensions: dataset.dimensions.includes("day") ? ["day"] : dataset.dimensions.slice(0, 1),
    measures: dataset.measures.slice(0, 1),
    filters: [],
    order_by: dataset.measures[0] ? [{ measure: dataset.measures[0], direction: "desc" }] : [],
    limit: 100,
  };
}

export default function ReportsPage() {
  usePageTitle("Reports");
  const { hasRole } = useOrg();
  const catalog = useReportCatalog();
  const savedReports = useSavedReports();
  const diagnostics = useReportingDiagnostics();
  const createSavedReport = useCreateSavedReport();
  const deleteSavedReport = useDeleteSavedReport();
  const runSavedReport = useRunSavedReport();
  const exportQuery = useExportReportQuery();
  const exportSaved = useExportSavedReport();
  const runProjector = useRunReportingProjector();
  const backfill = useBackfillReporting();
  const [selectedDataset, setSelectedDataset] = useState<string>("");
  const [selectedDimension, setSelectedDimension] = useState<string>("day");
  const [selectedMeasure, setSelectedMeasure] = useState<string>("");
  const [selectedRange, setSelectedRange] = useState("7");
  const [saveName, setSaveName] = useState("");
  const [manualResult, setManualResult] = useState<ReportResult>();

  const datasets = useMemo(() => catalog.data?.datasets ?? [], [catalog.data?.datasets]);
  const dataset = useMemo(
    () => datasets.find((entry) => entry.name === selectedDataset) ?? datasets[0],
    [datasets, selectedDataset],
  );

  useEffect(() => {
    if (!dataset || selectedDataset) return;
    setSelectedDataset(dataset.name);
    setSelectedDimension(
      dataset.dimensions.includes("day") ? "day" : (dataset.dimensions[0] ?? "none"),
    );
    setSelectedMeasure(dataset.measures[0] ?? "");
  }, [dataset, selectedDataset]);

  const query = useMemo<ReportQuery | undefined>(() => {
    if (!dataset || !selectedMeasure) return defaultQuery(dataset, Number(selectedRange));
    return {
      dataset: dataset.name,
      time_range: range(Number(selectedRange)),
      dimensions: selectedDimension === "none" ? [] : [selectedDimension],
      measures: [selectedMeasure],
      filters: [],
      order_by: [{ measure: selectedMeasure, direction: "desc" }],
      limit: 100,
    };
  }, [dataset, selectedDimension, selectedMeasure, selectedRange]);

  const result = useReportQuery(query);
  const activeResult = manualResult ?? result.data;
  const activeColumns = activeResult?.columns ?? [];
  const activeRows = activeResult?.rows ?? [];
  const measureColumn = activeColumns.find((column) => column.kind === "measure")?.name;
  const canAdministerReports = hasRole("admin");
  const measureTotal = measureColumn
    ? activeRows.reduce(
        (sum, row) => sum + (typeof row[measureColumn] === "number" ? row[measureColumn] : 0),
        0,
      )
    : activeRows.length;

  const handleDatasetChange = (value: string) => {
    const next = datasets.find((entry) => entry.name === value);
    setManualResult(undefined);
    setSelectedDataset(value);
    setSelectedDimension(
      next?.dimensions.includes("day") ? "day" : (next?.dimensions[0] ?? "none"),
    );
    setSelectedMeasure(next?.measures[0] ?? "");
  };

  const handleSave = async () => {
    if (!query || !saveName.trim()) return;
    await createSavedReport.mutateAsync({
      name: saveName.trim(),
      query,
      dashboard: { chart_type: "table" },
    });
    setSaveName("");
  };

  const handleRunSaved = async (report: SavedReport) => {
    setSelectedDataset(report.query.dataset);
    setSelectedDimension(report.query.dimensions[0] ?? "none");
    setSelectedMeasure(report.query.measures[0] ?? "");
    setManualResult(await runSavedReport.mutateAsync(report.id));
  };

  const handleExportCurrent = async () => {
    if (!query) return;
    saveExport(await exportQuery.mutateAsync({ query, format: "csv" }));
  };

  if (catalog.isLoading) {
    return (
      <PageShell fullWidth>
        <PageHeader title="Reports" />
        <PageBody>
          <div className="grid gap-4 md:grid-cols-4">
            {[...Array(4)].map((_, index) => (
              <Skeleton key={index} className="h-28" />
            ))}
          </div>
          <Skeleton className="h-96" />
        </PageBody>
      </PageShell>
    );
  }

  return (
    <PageShell fullWidth>
      <PageHeader
        title="Reports"
        description="Org-scoped analytics from reporting projections."
        actions={
          <>
            {canAdministerReports && (
              <>
                <Button
                  variant="outline"
                  onClick={() => backfill.mutate()}
                  disabled={backfill.isPending}
                >
                  <RefreshCw className="h-4 w-4 mr-2" />
                  Backfill
                </Button>
                <Button
                  variant="outline"
                  onClick={() => runProjector.mutate()}
                  disabled={runProjector.isPending}
                >
                  <RotateCw className="h-4 w-4 mr-2" />
                  Project
                </Button>
              </>
            )}
            <Button onClick={handleExportCurrent} disabled={!query || exportQuery.isPending}>
              <Download className="h-4 w-4 mr-2" />
              CSV
            </Button>
          </>
        }
      />

      <PageBody className="space-y-6">
        <div className="grid gap-4 md:grid-cols-4">
          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm font-medium">Rows</CardTitle>
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-semibold">{activeRows.length.toLocaleString()}</div>
              <p className="text-xs text-muted-foreground">Limit {query?.limit ?? 100}</p>
            </CardContent>
          </Card>
          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm font-medium">{measureColumn ?? "Measure"}</CardTitle>
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-semibold">{formatValue(measureTotal)}</div>
              <p className="text-xs text-muted-foreground">
                {formatDatasetName(query?.dataset ?? "")}
              </p>
            </CardContent>
          </Card>
          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm font-medium">Freshness</CardTitle>
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-semibold">
                {freshnessLabel(activeResult?.freshness_lag_ms)}
              </div>
              <p className="text-xs text-muted-foreground">
                {activeResult?.as_of ? new Date(activeResult.as_of).toLocaleTimeString() : "-"}
              </p>
            </CardContent>
          </Card>
          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm font-medium">Outbox</CardTitle>
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-semibold">
                {(diagnostics.data?.outbox.pending ?? 0).toLocaleString()}
              </div>
              <p className="text-xs text-muted-foreground">
                {diagnostics.data?.outbox.failed ?? 0} failed,{" "}
                {diagnostics.data?.outbox.processing ?? 0} processing
              </p>
            </CardContent>
          </Card>
        </div>

        <section className="grid gap-6 xl:grid-cols-[minmax(0,1fr)_360px]">
          <div className="space-y-4">
            <Card>
              <CardHeader>
                <CardTitle className="flex items-center gap-2 text-base">
                  <BarChart3 className="h-5 w-5" />
                  Query
                </CardTitle>
              </CardHeader>
              <CardContent className="grid gap-4 md:grid-cols-5">
                <div className="space-y-2">
                  <Label>Dataset</Label>
                  <Select value={dataset?.name ?? ""} onValueChange={handleDatasetChange}>
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {datasets.map((entry) => (
                        <SelectItem key={entry.name} value={entry.name}>
                          {formatDatasetName(entry.name)}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-2">
                  <Label>Dimension</Label>
                  <Select
                    value={selectedDimension}
                    onValueChange={(value) => {
                      setManualResult(undefined);
                      setSelectedDimension(value);
                    }}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="none">None</SelectItem>
                      {dataset?.dimensions.map((name) => (
                        <SelectItem key={name} value={name}>
                          {name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-2">
                  <Label>Measure</Label>
                  <Select
                    value={selectedMeasure}
                    onValueChange={(value) => {
                      setManualResult(undefined);
                      setSelectedMeasure(value);
                    }}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {dataset?.measures.map((name) => (
                        <SelectItem key={name} value={name}>
                          {name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-2">
                  <Label>Range</Label>
                  <Select
                    value={selectedRange}
                    onValueChange={(value) => {
                      setManualResult(undefined);
                      setSelectedRange(value);
                    }}
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {RANGE_OPTIONS.map((option) => (
                        <SelectItem key={option.value} value={option.value}>
                          {option.label}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-2">
                  <Label>Save</Label>
                  <div className="flex gap-2">
                    <Input
                      value={saveName}
                      onChange={(event) => setSaveName(event.target.value)}
                      placeholder="Report name"
                    />
                    <Button
                      variant="outline"
                      size="icon"
                      onClick={handleSave}
                      disabled={!saveName.trim() || createSavedReport.isPending}
                      aria-label="Save report"
                    >
                      <Save className="h-4 w-4" />
                    </Button>
                  </div>
                </div>
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle className="flex items-center gap-2 text-base">
                  <Database className="h-5 w-5" />
                  Result
                  {result.isFetching && <Badge variant="outline">Loading</Badge>}
                </CardTitle>
              </CardHeader>
              <CardContent>
                {result.error ? (
                  <div className="border border-destructive/30 bg-destructive/10 p-4 text-sm text-destructive">
                    {result.error.message}
                  </div>
                ) : activeRows.length === 0 ? (
                  <div className="border p-8 text-center text-sm text-muted-foreground">
                    No rows for this query.
                  </div>
                ) : (
                  <Table>
                    <TableHeader>
                      <TableRow>
                        {activeColumns.map((column) => (
                          <TableHead key={column.name}>{column.name}</TableHead>
                        ))}
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {activeRows.map((row, rowIndex) => (
                        <TableRow key={rowIndex}>
                          {activeColumns.map((column) => (
                            <TableCell key={column.name}>{formatValue(row[column.name])}</TableCell>
                          ))}
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                )}
              </CardContent>
            </Card>
          </div>

          <aside className="space-y-4">
            <Card>
              <CardHeader>
                <CardTitle className="text-base">Saved Reports</CardTitle>
              </CardHeader>
              <CardContent className="space-y-2">
                {(savedReports.data ?? []).length === 0 ? (
                  <p className="text-sm text-muted-foreground">No saved reports.</p>
                ) : (
                  savedReports.data?.map((report) => (
                    <div key={report.id} className="flex items-center justify-between border p-2">
                      <button
                        type="button"
                        className="min-w-0 text-left"
                        onClick={() => handleRunSaved(report)}
                      >
                        <p className="truncate text-sm font-medium">{report.name}</p>
                        <p className="text-xs text-muted-foreground">
                          {formatDatasetName(report.query.dataset)}
                        </p>
                      </button>
                      <div className="flex shrink-0 items-center gap-1">
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => handleRunSaved(report)}
                          aria-label="Run saved report"
                        >
                          <Play className="h-4 w-4" />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={async () =>
                            saveExport(
                              await exportSaved.mutateAsync({ reportId: report.id, format: "csv" }),
                            )
                          }
                          aria-label="Export saved report"
                        >
                          <Download className="h-4 w-4" />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon"
                          onClick={() => deleteSavedReport.mutate(report.id)}
                          aria-label="Delete saved report"
                        >
                          <Trash2 className="h-4 w-4" />
                        </Button>
                      </div>
                    </div>
                  ))
                )}
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle className="text-base">Projection Lag</CardTitle>
              </CardHeader>
              <CardContent>
                <div className="space-y-2">
                  {(diagnostics.data?.projector_lag ?? []).map((lag) => (
                    <div
                      key={lag.dataset}
                      className="flex items-center justify-between border-b py-1.5 last:border-b-0"
                    >
                      <span className="text-sm">{formatDatasetName(lag.dataset)}</span>
                      <Badge variant={lag.freshness_lag_ms === null ? "outline" : "secondary"}>
                        {freshnessLabel(lag.freshness_lag_ms)}
                      </Badge>
                    </div>
                  ))}
                </div>
              </CardContent>
            </Card>

            {diagnostics.data?.outbox.failed_rows.length ? (
              <Card>
                <CardHeader>
                  <CardTitle className="text-base">Failed Rows</CardTitle>
                </CardHeader>
                <CardContent className="space-y-2">
                  {diagnostics.data.outbox.failed_rows.slice(0, 5).map((row) => (
                    <div key={row.id} className="border p-2 text-xs">
                      <div className="font-medium">{row.source_type}</div>
                      <div className="truncate text-muted-foreground">{row.source_id}</div>
                      {row.last_error && (
                        <div className="mt-1 text-destructive">{row.last_error}</div>
                      )}
                    </div>
                  ))}
                </CardContent>
              </Card>
            ) : null}
          </aside>
        </section>
      </PageBody>
    </PageShell>
  );
}
