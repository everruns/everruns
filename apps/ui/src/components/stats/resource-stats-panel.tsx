"use client";

import { BarChart3, Clock, DollarSign, Play, Zap } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import type { ResourceStats } from "@/lib/api/types";
import { formatCompactNumber, formatCostUsd, formatDate, formatTokens } from "@/lib/formatting";

interface ResourceStatsPanelProps {
  stats: ResourceStats | undefined;
  isLoading: boolean;
  error: unknown;
}

function formatDuration(ms: number | null | undefined): string {
  if (!ms || ms <= 0) return "0s";
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const remainingMinutes = minutes % 60;
  if (hours < 24) return remainingMinutes > 0 ? `${hours}h ${remainingMinutes}m` : `${hours}h`;
  const days = Math.floor(hours / 24);
  const remainingHours = hours % 24;
  return remainingHours > 0 ? `${days}d ${remainingHours}h` : `${days}d`;
}

function formatOptionalDate(value: string | null | undefined): string {
  return value ? formatDate(value) : "Never";
}

function StatCard({
  title,
  value,
  detail,
  icon: Icon,
}: {
  title: string;
  value: string;
  detail: string;
  icon: typeof BarChart3;
}) {
  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
        <CardTitle className="text-sm font-medium">{title}</CardTitle>
        <Icon className="h-4 w-4 text-muted-foreground" />
      </CardHeader>
      <CardContent>
        <div className="text-2xl font-bold">{value}</div>
        <p className="text-xs text-muted-foreground">{detail}</p>
      </CardContent>
    </Card>
  );
}

export function ResourceStatsPanel({ stats, isLoading, error }: ResourceStatsPanelProps) {
  if (isLoading) {
    return (
      <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3 xl:grid-cols-5">
        {Array.from({ length: 5 }).map((_, index) => (
          <Skeleton key={index} className="h-28 w-full" />
        ))}
      </div>
    );
  }

  if (error) {
    return (
      <Card>
        <CardContent className="py-8 text-center text-sm text-destructive">
          Failed to load stats.
        </CardContent>
      </Card>
    );
  }

  if (!stats) return null;

  const totalTokens =
    stats.total_input_tokens +
    stats.total_output_tokens +
    stats.total_cache_read_tokens +
    stats.total_cache_creation_tokens;
  const executionsPerSession =
    stats.session_count > 0 ? stats.execution_count / stats.session_count : 0;

  return (
    <div className="space-y-6">
      <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3 xl:grid-cols-5">
        <StatCard
          title="Sessions"
          value={formatCompactNumber(stats.session_count)}
          detail={`${formatCompactNumber(stats.active_session_count)} active / ${formatCompactNumber(
            stats.idle_session_count,
          )} idle`}
          icon={BarChart3}
        />
        <StatCard
          title="Executions"
          value={formatCompactNumber(stats.execution_count)}
          detail={`${executionsPerSession.toFixed(1)} per session`}
          icon={Play}
        />
        <StatCard
          title="Session Time"
          value={formatDuration(stats.total_session_duration_ms)}
          detail={`Avg ${formatDuration(stats.avg_session_duration_ms)}`}
          icon={Clock}
        />
        <StatCard
          title="Tokens"
          value={formatTokens(totalTokens)}
          detail={`${formatTokens(stats.total_input_tokens)} input / ${formatTokens(
            stats.total_output_tokens,
          )} output`}
          icon={Zap}
        />
        <StatCard
          title="Cost"
          value={formatCostUsd(stats.total_cost_usd)}
          detail={`${formatCostUsd(stats.total_actual_cost_usd)} actual / ${formatCostUsd(
            stats.total_estimated_cost_usd,
          )} est.`}
          icon={DollarSign}
        />
      </div>

      <div className="grid gap-6 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle>Session Status</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="flex items-center justify-between text-sm">
              <span className="text-muted-foreground">Started</span>
              <span className="font-medium">
                {formatCompactNumber(stats.started_session_count)}
              </span>
            </div>
            <div className="flex items-center justify-between text-sm">
              <span className="text-muted-foreground">Active</span>
              <span className="font-medium">{formatCompactNumber(stats.active_session_count)}</span>
            </div>
            <div className="flex items-center justify-between text-sm">
              <span className="text-muted-foreground">Waiting for tools</span>
              <span className="font-medium">
                {formatCompactNumber(stats.waiting_for_tool_results_session_count)}
              </span>
            </div>
            <div className="flex items-center justify-between text-sm">
              <span className="text-muted-foreground">Idle</span>
              <span className="font-medium">{formatCompactNumber(stats.idle_session_count)}</span>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Activity</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="flex items-center justify-between gap-4 text-sm">
              <span className="text-muted-foreground">First session</span>
              <span className="text-right font-medium">
                {formatOptionalDate(stats.first_session_at)}
              </span>
            </div>
            <div className="flex items-center justify-between gap-4 text-sm">
              <span className="text-muted-foreground">Latest session</span>
              <span className="text-right font-medium">
                {formatOptionalDate(stats.last_session_at)}
              </span>
            </div>
            <div className="flex items-center justify-between gap-4 text-sm">
              <span className="text-muted-foreground">Latest execution</span>
              <span className="text-right font-medium">
                {formatOptionalDate(stats.last_execution_at)}
              </span>
            </div>
            <div className="flex items-center justify-between gap-4 text-sm">
              <span className="text-muted-foreground">Cached tokens</span>
              <span className="text-right font-medium">
                {formatTokens(stats.total_cache_read_tokens + stats.total_cache_creation_tokens)}
              </span>
            </div>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
