"use client";

import Link from "next/link";
import { Badge } from "@/components/ui/badge";
import { Button, buttonVariants } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Activity, Eye, Copy, Check } from "lucide-react";
import { cn } from "@/lib/utils";
import { useState } from "react";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useSessionContext } from "../session-context";

function CopyButton({ data }: { data: unknown }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(JSON.stringify(data, null, 2));
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.error("Failed to copy:", err);
    }
  };

  return (
    <Button variant="ghost" size="sm" onClick={handleCopy} className="h-6 px-2 text-xs">
      {copied ? (
        <>
          <Check className="w-3 h-3 mr-1" />
          Copied
        </>
      ) : (
        <>
          <Copy className="w-3 h-3 mr-1" />
          Copy
        </>
      )}
    </Button>
  );
}

export default function EventsPage() {
  const { events, eventsLoading, agentId, sessionId } = useSessionContext();

  const basePath = `/agents/${agentId}/sessions/${sessionId}`;

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
                <TableRow key={event.id} className="align-top">
                  <TableCell className="font-mono text-xs">{event.sequence}</TableCell>
                  <TableCell>
                    <Badge variant="outline" className="font-mono text-xs">
                      {event.type}
                    </Badge>
                  </TableCell>
                  <TableCell className="text-xs text-muted-foreground">
                    {new Date(event.ts).toLocaleString()}
                  </TableCell>
                  <TableCell className="font-mono text-xs">
                    <div className="relative">
                      <div className="absolute right-0 top-0 z-10 flex items-center gap-1 bg-background/80 backdrop-blur-sm rounded px-1">
                        {event.type === "llm.generation" && (
                          <Link
                            href={`${basePath}/llm-history/${event.id}`}
                            className={cn(buttonVariants({ variant: "ghost", size: "sm" }), "h-6 px-2 text-xs")}
                          >
                            <Eye className="w-3 h-3 mr-1" />
                            View
                          </Link>
                        )}
                        <CopyButton data={event.data} />
                      </div>
                      <pre className="whitespace-pre-wrap break-all text-xs bg-muted p-2 rounded max-h-[200px] overflow-y-auto">
                        {JSON.stringify(event.data, null, 2)}
                      </pre>
                    </div>
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
