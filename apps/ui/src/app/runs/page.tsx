"use client";

import Link from "next/link";
import { useRuns } from "@/hooks/use-runs";
import { useAgents } from "@/hooks/use-agents";
import { Header } from "@/components/layout/header";
import { RunStatusBadge } from "@/components/runs/run-status-badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Play, RefreshCw, ExternalLink } from "lucide-react";
import { useState } from "react";
import type { RunStatus } from "@/lib/api/types";

export default function RunsPage() {
  const [statusFilter, setStatusFilter] = useState<string>("all");
  const [agentFilter, setAgentFilter] = useState<string>("all");

  const { data: runs = [], isLoading: runsLoading, refetch } = useRuns({
    status: statusFilter !== "all" ? statusFilter : undefined,
    agent_id: agentFilter !== "all" ? agentFilter : undefined,
  });
  const { data: agents = [] } = useAgents();

  const getAgentName = (agentId: string) => {
    const agent = agents.find((a) => a.id === agentId);
    return agent?.name || "Unknown Agent";
  };

  const formatDate = (dateStr: string) => {
    return new Date(dateStr).toLocaleString();
  };

  // Calculate duration - uses Date.now() for running tasks to show live duration
  const formatDuration = (startedAt: string | null, finishedAt: string | null) => {
    if (!startedAt) return "-";
    const start = new Date(startedAt).getTime();
    // eslint-disable-next-line react-hooks/purity
    const end = finishedAt ? new Date(finishedAt).getTime() : Date.now();
    const seconds = Math.floor((end - start) / 1000);

    if (seconds < 60) return `${seconds}s`;
    const minutes = Math.floor(seconds / 60);
    const remainingSeconds = seconds % 60;
    return `${minutes}m ${remainingSeconds}s`;
  };

  return (
    <>
      <Header
        title="Runs"
        action={
          <Button variant="outline" onClick={() => refetch()}>
            <RefreshCw className="h-4 w-4 mr-2" />
            Refresh
          </Button>
        }
      />
      <div className="p-6 space-y-6">
        {/* Filters */}
        <Card>
          <CardHeader>
            <CardTitle className="text-base">Filters</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="flex gap-4">
              <div className="w-48">
                <Select value={statusFilter} onValueChange={setStatusFilter}>
                  <SelectTrigger>
                    <SelectValue placeholder="All statuses" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">All Statuses</SelectItem>
                    <SelectItem value="pending">Pending</SelectItem>
                    <SelectItem value="running">Running</SelectItem>
                    <SelectItem value="completed">Completed</SelectItem>
                    <SelectItem value="failed">Failed</SelectItem>
                    <SelectItem value="cancelled">Cancelled</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <div className="w-48">
                <Select value={agentFilter} onValueChange={setAgentFilter}>
                  <SelectTrigger>
                    <SelectValue placeholder="All agents" />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="all">All Agents</SelectItem>
                    {agents.map((agent) => (
                      <SelectItem key={agent.id} value={agent.id}>
                        {agent.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>
          </CardContent>
        </Card>

        {/* Runs Table */}
        <Card>
          <CardContent className="p-0">
            {runsLoading ? (
              <div className="p-6 space-y-4">
                {[...Array(5)].map((_, i) => (
                  <Skeleton key={i} className="h-12 w-full" />
                ))}
              </div>
            ) : runs.length === 0 ? (
              <div className="text-center py-12">
                <Play className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
                <h3 className="text-lg font-medium mb-2">No runs found</h3>
                <p className="text-muted-foreground">
                  {statusFilter !== "all" || agentFilter !== "all"
                    ? "Try adjusting your filters"
                    : "Runs will appear here when agents are executed"}
                </p>
              </div>
            ) : (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Run ID</TableHead>
                    <TableHead>Agent</TableHead>
                    <TableHead>Status</TableHead>
                    <TableHead>Created</TableHead>
                    <TableHead>Duration</TableHead>
                    <TableHead className="w-12"></TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {runs.map((run) => (
                    <TableRow key={run.id}>
                      <TableCell className="font-mono text-sm">
                        {run.id.slice(0, 8)}...
                      </TableCell>
                      <TableCell>
                        <Link
                          href={`/agents/${run.agent_id}`}
                          className="hover:underline"
                        >
                          {getAgentName(run.agent_id)}
                        </Link>
                      </TableCell>
                      <TableCell>
                        <RunStatusBadge status={run.status as RunStatus} />
                      </TableCell>
                      <TableCell className="text-sm text-muted-foreground">
                        {formatDate(run.created_at)}
                      </TableCell>
                      <TableCell className="text-sm">
                        {formatDuration(run.started_at, run.finished_at)}
                      </TableCell>
                      <TableCell>
                        <Link href={`/runs/${run.id}`}>
                          <Button variant="ghost" size="sm">
                            <ExternalLink className="h-4 w-4" />
                          </Button>
                        </Link>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            )}
          </CardContent>
        </Card>
      </div>
    </>
  );
}
