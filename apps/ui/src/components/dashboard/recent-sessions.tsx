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
import { Badge } from "@/components/ui/badge";
import type { Session, Harness } from "@/lib/api/types";

interface RecentSessionsProps {
  sessions: Session[];
  harnesses: Harness[];
}

export function RecentSessions({ sessions, harnesses }: RecentSessionsProps) {
  const recentSessions = sessions.slice(0, 10);
  const harnessMap = new Map(harnesses.map((h) => [h.id, h]));

  const formatDate = (date: string) => {
    return new Date(date).toLocaleString();
  };

  const getDuration = (session: Session) => {
    if (!session.started_at || !session.finished_at) return "-";
    const start = new Date(session.started_at).getTime();
    const end = new Date(session.finished_at).getTime();
    const seconds = Math.round((end - start) / 1000);
    if (seconds < 60) return `${seconds}s`;
    return `${Math.round(seconds / 60)}m ${seconds % 60}s`;
  };

  const getStatusBadge = (session: Session) => {
    if (session.finished_at) {
      return <Badge variant="outline" className="bg-green-100 text-green-800">Completed</Badge>;
    }
    if (session.started_at) {
      return <Badge variant="default">Running</Badge>;
    }
    return <Badge variant="secondary">Pending</Badge>;
  };

  return (
    <Card>
      <CardHeader>
        <CardTitle>Recent Sessions</CardTitle>
      </CardHeader>
      <CardContent>
        {recentSessions.length === 0 ? (
          <p className="text-center text-muted-foreground py-8">
            No sessions yet. Create a harness and start a session to begin.
          </p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Session</TableHead>
                <TableHead>Harness</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Started</TableHead>
                <TableHead>Duration</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {recentSessions.map((session) => {
                const harness = harnessMap.get(session.harness_id);
                return (
                  <TableRow key={session.id}>
                    <TableCell>
                      <Link
                        href={`/harnesses/${session.harness_id}/sessions/${session.id}`}
                        className="font-mono text-sm hover:underline"
                      >
                        {session.title || session.id.slice(0, 8) + "..."}
                      </Link>
                    </TableCell>
                    <TableCell>
                      <Link
                        href={`/harnesses/${session.harness_id}`}
                        className="hover:underline"
                      >
                        {harness?.display_name || "Unknown"}
                      </Link>
                    </TableCell>
                    <TableCell>
                      {getStatusBadge(session)}
                    </TableCell>
                    <TableCell className="text-sm text-muted-foreground">
                      {session.started_at ? formatDate(session.started_at) : "-"}
                    </TableCell>
                    <TableCell className="text-sm text-muted-foreground">
                      {getDuration(session)}
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        )}
      </CardContent>
    </Card>
  );
}
