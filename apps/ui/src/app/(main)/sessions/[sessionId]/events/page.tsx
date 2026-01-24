"use client";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Activity, Copy, Check, RefreshCw, ChevronLeft, ChevronRight } from "lucide-react";
import { useState, useMemo, useEffect, useRef } from "react";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useSessionContext } from "../session-context";

const EVENTS_PER_PAGE = 50;

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
  const { events, eventsLoading, sessionId } = useSessionContext();
  const [currentPage, setCurrentPage] = useState(1);
  const [hasNewEvents, setHasNewEvents] = useState(false);
  const lastKnownCountRef = useRef(0);

  // Sort events by sequence descending (newest first)
  const sortedEvents = useMemo(() => {
    if (!events) return [];
    return [...events].sort((a, b) => (b.sequence ?? 0) - (a.sequence ?? 0));
  }, [events]);

  // Calculate pagination
  const totalEvents = sortedEvents.length;
  const totalPages = Math.ceil(totalEvents / EVENTS_PER_PAGE);
  const startIndex = (currentPage - 1) * EVENTS_PER_PAGE;
  const endIndex = startIndex + EVENTS_PER_PAGE;
  const paginatedEvents = sortedEvents.slice(startIndex, endIndex);

  // Track new events arriving while viewing the list
  useEffect(() => {
    if (!events) return;

    const currentCount = events.length;

    // Initialize on first load
    if (lastKnownCountRef.current === 0) {
      lastKnownCountRef.current = currentCount;
      return;
    }

    // New events arrived while user is viewing
    if (currentCount > lastKnownCountRef.current && currentPage === 1) {
      // If on first page, auto-update the count (they'll see new events)
      lastKnownCountRef.current = currentCount;
    } else if (currentCount > lastKnownCountRef.current) {
      // If on a different page, show refresh button
      setHasNewEvents(true);
    }
  }, [events, currentPage]);

  // Reset to page 1 when session changes
  useEffect(() => {
    setCurrentPage(1);
    setHasNewEvents(false);
    lastKnownCountRef.current = 0;
  }, [sessionId]);

  const handleRefresh = () => {
    setCurrentPage(1);
    setHasNewEvents(false);
    lastKnownCountRef.current = events?.length ?? 0;
  };

  const handlePageChange = (page: number) => {
    setCurrentPage(page);
  };

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
        <div className="space-y-4">
          {/* Header with refresh button and pagination info */}
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              {hasNewEvents && (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={handleRefresh}
                  className="gap-1.5 text-primary border-primary hover:bg-primary/10"
                >
                  <RefreshCw className="w-3.5 h-3.5" />
                  New events available
                </Button>
              )}
            </div>
            <div className="text-sm text-muted-foreground">
              Showing {startIndex + 1}-{Math.min(endIndex, totalEvents)} of {totalEvents} events
            </div>
          </div>

          {/* Events table */}
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
                {paginatedEvents.map((event) => (
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

          {/* Pagination controls */}
          {totalPages > 1 && (
            <div className="flex items-center justify-center gap-2">
              <Button
                variant="outline"
                size="sm"
                onClick={() => handlePageChange(currentPage - 1)}
                disabled={currentPage === 1}
              >
                <ChevronLeft className="w-4 h-4" />
              </Button>
              <div className="flex items-center gap-1">
                {/* Show page numbers with ellipsis for large page counts */}
                {Array.from({ length: totalPages }, (_, i) => i + 1)
                  .filter((page) => {
                    // Always show first, last, and pages around current
                    if (page === 1 || page === totalPages) return true;
                    if (Math.abs(page - currentPage) <= 1) return true;
                    return false;
                  })
                  .map((page, idx, arr) => (
                    <span key={page} className="flex items-center">
                      {idx > 0 && arr[idx - 1] !== page - 1 && (
                        <span className="px-2 text-muted-foreground">...</span>
                      )}
                      <Button
                        variant={currentPage === page ? "default" : "outline"}
                        size="sm"
                        onClick={() => handlePageChange(page)}
                        className="w-8 h-8 p-0"
                      >
                        {page}
                      </Button>
                    </span>
                  ))}
              </div>
              <Button
                variant="outline"
                size="sm"
                onClick={() => handlePageChange(currentPage + 1)}
                disabled={currentPage === totalPages}
              >
                <ChevronRight className="w-4 h-4" />
              </Button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
