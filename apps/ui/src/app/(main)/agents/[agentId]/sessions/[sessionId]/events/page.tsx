"use client";

import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Activity } from "lucide-react";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useSessionContext } from "../session-context";

export default function EventsPage() {
  const { events, eventsLoading } = useSessionContext();

  return (
    <div className="flex-1 overflow-y-auto p-4">
      {eventsLoading ? (
        <div className="space-y-2">
          <Skeleton className="h-8 w-full" />
          <Skeleton className="h-8 w-full" />
          <Skeleton className="h-8 w-full" />
        </div>
      ) : events?.length === 0 ? (
        <div className="flex flex-col items-center justify-center h-full text-center text-muted-foreground">
          <Activity className="w-12 h-12 mb-4 opacity-50" />
          <p className="text-lg font-medium">No events yet</p>
          <p className="text-sm">Events will appear here as the session runs</p>
        </div>
      ) : (
        <div className="border rounded-lg">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-[80px]">Seq</TableHead>
                <TableHead className="w-[180px]">Type</TableHead>
                <TableHead className="w-[200px]">Timestamp</TableHead>
                <TableHead>Data</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {events?.map((event) => (
                <TableRow key={event.id}>
                  <TableCell className="font-mono text-xs">{event.sequence}</TableCell>
                  <TableCell>
                    <Badge variant="outline" className="font-mono text-xs">
                      {event.type}
                    </Badge>
                  </TableCell>
                  <TableCell className="text-xs text-muted-foreground">
                    {new Date(event.ts).toLocaleString()}
                  </TableCell>
                  <TableCell className="font-mono text-xs max-w-[500px]">
                    <pre className="whitespace-pre-wrap break-all text-xs bg-muted p-2 rounded max-h-[200px] overflow-y-auto">
                      {JSON.stringify(event.data, null, 2)}
                    </pre>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}
    </div>
  );
}
