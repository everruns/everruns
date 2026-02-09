"use client";

import { Skeleton } from "@/components/ui/skeleton";
import { useSessionContext } from "../session-context";
import { TrajectoryView } from "@/components/trajectory/trajectory-view";

export default function TrajectoryPage() {
  const { events, eventsLoading } = useSessionContext();

  if (eventsLoading) {
    return (
      <div className="flex-1 p-4 space-y-4">
        <Skeleton className="h-8 w-1/4" />
        <Skeleton className="h-[calc(100%-3rem)] w-full" />
      </div>
    );
  }

  return (
    <div className="flex-1 overflow-hidden">
      <TrajectoryView events={events ?? []} />
    </div>
  );
}
