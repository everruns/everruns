"use client";

import Link from "next/link";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { RunStatusBadge } from "@/components/runs/run-status-badge";
import type { Run, Agent } from "@/lib/api/types";

interface RecentRunsProps {
  runs: Run[];
  agents: Agent[];
}

export function RecentRuns({ runs, agents }: RecentRunsProps) {
  const recentRuns = runs.slice(0, 10);
  const agentMap = new Map(agents.map((a) => [a.id, a]));

  const formatDate = (date: string) => {
    return new Date(date).toLocaleString();
  };

  const getDuration = (run: Run) => {
    if (!run.started_at || !run.finished_at) return "-";
    const start = new Date(run.started_at).getTime();
    const end = new Date(run.finished_at).getTime();
    const seconds = Math.round((end - start) / 1000);
    if (seconds < 60) return `${seconds}s`;
    return `${Math.round(seconds / 60)}m ${seconds % 60}s`;
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>Recent Runs</CardTitle>
      </CardHeader>
      <CardContent>
        {recentRuns.length === 0 ? (
          <p className="text-center text-muted-foreground py-8">
            No runs yet. Start a chat or create a run to get started.
          </p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>ID</TableHead>
                <TableHead>Agent</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Started</TableHead>
                <TableHead>Duration</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {recentRuns.map((run) => (
                <TableRow key={run.id}>
                  <TableCell>
                    <Link
                      href={`/runs/${run.id}`}
                      className="font-mono text-sm hover:underline"
                    >
                      {run.id.slice(0, 8)}...
                    </Link>
                  </TableCell>
                  <TableCell>
                    {agentMap.get(run.agent_id)?.name || "Unknown"}
                  </TableCell>
                  <TableCell>
                    <RunStatusBadge status={run.status} />
                  </TableCell>
                  <TableCell className="text-sm text-muted-foreground">
                    {run.started_at ? formatDate(run.started_at) : "-"}
                  </TableCell>
                  <TableCell className="text-sm text-muted-foreground">
                    {getDuration(run)}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </CardContent>
    </Card>
  );
}
