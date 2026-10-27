"use client";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { CopyButton } from "@/components/ui/copy-button";
import { Skeleton } from "@/components/ui/skeleton";
import { usePageTitle, useRefreshSystemDoctor, useSystemDoctor } from "@/hooks";
import type { CheckStatus, DoctorCheck, DoctorReport } from "@/lib/api/types";
import { formatDistanceToNow } from "@/lib/formatting";
import {
  CHECK_AREA_LABELS,
  CHECK_STATUS_LABELS,
  doctorReportText,
  doctorVerdict,
  groupDoctorChecks,
} from "@/lib/system-doctor";
import { cn } from "@/lib/utils";
import {
  AlertTriangle,
  CheckCircle2,
  Info,
  RefreshCw,
  XCircle,
} from "lucide-react";

function StatusIcon({
  status,
  className,
}: {
  status: CheckStatus;
  className?: string;
}) {
  switch (status) {
    case "problem":
      return (
        <XCircle
          className={cn("h-4 w-4 text-destructive", className)}
          aria-hidden
        />
      );
    case "warning":
      return (
        <AlertTriangle
          className={cn("h-4 w-4 text-warning", className)}
          aria-hidden
        />
      );
    case "ok":
      return (
        <CheckCircle2
          className={cn("h-4 w-4 text-success", className)}
          aria-hidden
        />
      );
    default:
      return (
        <Info
          className={cn("h-4 w-4 text-muted-foreground", className)}
          aria-hidden
        />
      );
  }
}

function Verdict({ report }: { report: DoctorReport }) {
  const tone =
    report.status === "problem"
      ? "border-destructive/50 bg-destructive/5"
      : report.status === "warning"
        ? "border-warning/50 bg-warning/5"
        : "border-success/40 bg-success/5";
  return (
    <Card className={tone} data-testid="doctor-verdict">
      <CardContent className="flex items-start gap-3 py-4">
        <StatusIcon status={report.status} className="mt-0.5 h-5 w-5" />
        <div className="min-w-0 space-y-1">
          <p className="font-medium">{doctorVerdict(report)}</p>
          {report.setup.length > 0 && (
            <p className="text-sm text-muted-foreground">
              {report.setup.join(" · ")}
            </p>
          )}
          <p className="text-xs text-muted-foreground">
            Version {report.version} · grade {report.grade} · sign-in{" "}
            {report.auth_mode} · checked{" "}
            {formatDistanceToNow(new Date(report.checked_at), {
              addSuffix: true,
            })}
          </p>
        </div>
      </CardContent>
    </Card>
  );
}

function AttentionCheck({ check }: { check: DoctorCheck }) {
  return (
    <li className="space-y-1 py-3">
      <div className="flex flex-wrap items-center gap-2">
        <StatusIcon status={check.status} />
        <span className="font-medium">{check.title}</span>
        <Badge variant="outline">{CHECK_AREA_LABELS[check.area]}</Badge>
        <Badge
          variant={check.status === "problem" ? "destructive" : "secondary"}
        >
          {CHECK_STATUS_LABELS[check.status]}
        </Badge>
      </div>
      <p className="text-sm">{check.summary}</p>
      {check.why && (
        <p className="text-sm text-muted-foreground">{check.why}</p>
      )}
      {check.fix && (
        <p className="text-sm">
          <span className="font-medium">Fix: </span>
          {check.fix}
        </p>
      )}
    </li>
  );
}

function AreaCheck({ check }: { check: DoctorCheck }) {
  return (
    <li className="flex items-start gap-2 py-2">
      <StatusIcon status={check.status} className="mt-0.5 shrink-0" />
      <div className="min-w-0">
        <span className="text-sm font-medium">{check.title}</span>
        <p
          className={cn(
            "text-sm",
            check.status === "info" && "text-muted-foreground",
          )}
        >
          {check.summary}
        </p>
      </div>
    </li>
  );
}

function Header({ report }: { report?: DoctorReport }) {
  const refresh = useRefreshSystemDoctor();
  return (
    <div className="operational-toolbar mb-6 flex flex-wrap items-center justify-between gap-3">
      <div>
        <h1 className="text-2xl font-bold">Health</h1>
        <p className="text-sm text-muted-foreground">
          Whether every part of this install is set up right and working.
        </p>
      </div>
      <div className="flex items-center gap-2">
        {report && (
          <CopyButton value={doctorReportText(report)} label="Copy report" />
        )}
        <Button
          variant="outline"
          size="sm"
          onClick={() => refresh.mutate()}
          disabled={refresh.isPending}
        >
          <RefreshCw
            className={cn("mr-2 h-4 w-4", refresh.isPending && "animate-spin")}
          />
          Check again
        </Button>
      </div>
    </div>
  );
}

export default function SystemHealthPage() {
  usePageTitle("Health", "System");
  const { data: report, isLoading, error } = useSystemDoctor();

  if (isLoading) {
    return (
      <div className="operational-page container mx-auto p-6">
        <Header />
        <div className="space-y-4">
          <Skeleton className="h-24" />
          <Skeleton className="h-64" />
        </div>
      </div>
    );
  }

  if (error || !report) {
    return (
      <div className="operational-page container mx-auto p-6">
        <Header />
        <Card>
          <CardContent className="flex flex-col items-center justify-center py-12 text-center">
            <XCircle className="mb-4 h-12 w-12 text-muted-foreground" />
            <h3 className="mb-2 text-lg font-medium">
              Could not load the health report
            </h3>
            <p className="max-w-md text-sm text-muted-foreground">
              {error instanceof Error
                ? error.message
                : "The server did not answer."}
            </p>
          </CardContent>
        </Card>
      </div>
    );
  }

  const { attention, byArea } = groupDoctorChecks(report.checks);
  return (
    <div className="operational-page container mx-auto p-6">
      <Header report={report} />
      <div className="space-y-6">
        <Verdict report={report} />
        {attention.length > 0 && (
          <Card>
            <CardHeader>
              <CardTitle className="text-base">Needs attention</CardTitle>
              <CardDescription>Problems first, then warnings.</CardDescription>
            </CardHeader>
            <CardContent>
              <ul className="divide-y divide-border">
                {attention.map((check) => (
                  <AttentionCheck key={check.id} check={check} />
                ))}
              </ul>
            </CardContent>
          </Card>
        )}
        <div className="grid gap-4 md:grid-cols-2">
          {byArea.map(({ area, checks }) => (
            <Card key={area}>
              <CardHeader className="pb-2">
                <CardTitle className="text-base">
                  {CHECK_AREA_LABELS[area]}
                </CardTitle>
              </CardHeader>
              <CardContent>
                <ul className="divide-y divide-border">
                  {checks.map((check) => (
                    <AreaCheck key={check.id} check={check} />
                  ))}
                </ul>
              </CardContent>
            </Card>
          ))}
        </div>
      </div>
    </div>
  );
}
