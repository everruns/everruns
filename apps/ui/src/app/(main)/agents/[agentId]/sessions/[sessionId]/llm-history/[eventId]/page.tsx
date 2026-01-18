"use client";

import { use, useMemo } from "react";
import Link from "next/link";
import { ArrowLeft, AlertCircle } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { LlmHistoryViewer } from "@/components/llm/llm-history-viewer";
import { useSessionContext } from "../../session-context";
import type { LlmGenerationData } from "@/lib/api/types";

interface LlmHistoryPageProps {
  params: Promise<{ agentId: string; sessionId: string; eventId: string }>;
}

export default function LlmHistoryPage({ params }: LlmHistoryPageProps) {
  const { agentId, sessionId, eventId } = use(params);
  const { events, eventsLoading } = useSessionContext();

  // Find the specific llm.generation event by ID
  const event = useMemo(() => {
    if (!events) return null;
    return events.find(
      (e) => e.id === eventId && e.type === "llm.generation"
    );
  }, [events, eventId]);

  const basePath = `/agents/${agentId}/sessions/${sessionId}`;

  if (eventsLoading) {
    return (
      <div className="flex-1 overflow-y-auto p-4">
        <Skeleton className="h-8 w-48 mb-4" />
        <Skeleton className="h-32 w-full mb-4" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (!event) {
    return (
      <div className="flex-1 overflow-y-auto p-4">
        <Link
          href={`${basePath}/events`}
          className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-4"
        >
          <ArrowLeft className="w-4 h-4 mr-2" />
          Back to Events
        </Link>

        <Card className="border-red-200 dark:border-red-800 bg-red-50 dark:bg-red-950/30">
          <CardHeader className="pb-2">
            <CardTitle className="flex items-center gap-2 text-red-700 dark:text-red-400">
              <AlertCircle className="h-4 w-4" />
              Event not found
            </CardTitle>
          </CardHeader>
          <CardContent>
            <p className="text-sm text-red-600 dark:text-red-300">
              The llm.generation event with ID &quot;{eventId.slice(0, 8)}...&quot; was not found.
              It may have been deleted or the ID is incorrect.
            </p>
          </CardContent>
        </Card>
      </div>
    );
  }

  const data = event.data as LlmGenerationData;

  return (
    <div className="flex-1 overflow-y-auto p-4">
      <Link
        href={`${basePath}/events`}
        className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-4"
      >
        <ArrowLeft className="w-4 h-4 mr-2" />
        Back to Events
      </Link>

      <h1 className="text-xl font-bold mb-4">LLM Generation History</h1>

      <LlmHistoryViewer
        data={data}
        eventId={event.id}
        timestamp={event.ts}
        maxHeight="calc(100vh - 280px)"
      />
    </div>
  );
}
