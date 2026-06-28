"use client";

import dynamic from "next/dynamic";
import { Skeleton } from "@/components/ui/skeleton";
import { useSessionContext } from "../session-context";

// `@xyflow/react` (+ its stylesheet) is heavy and only used on this tab. Load
// it lazily so it stays out of the initial session-route bundle.
const TrajectoryView = dynamic(
  () => import("@/components/trajectory/trajectory-view").then((m) => m.TrajectoryView),
  {
    ssr: false,
    loading: () => (
      <div className="flex-1 p-4 space-y-4">
        <Skeleton className="h-8 w-1/4" />
        <Skeleton className="h-[calc(100%-3rem)] w-full" />
      </div>
    ),
  },
);

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
