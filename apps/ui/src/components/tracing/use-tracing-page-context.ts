"use client";

import { useChatPageRows } from "@/components/chat-mode/page-view-state";
import type { TracingDestination } from "@/lib/api/tracing";

export function useTracingDestinationRows(
  destinations: readonly TracingDestination[],
  state: { loading?: boolean; error?: boolean } = {},
) {
  useChatPageRows(
    "Tracing destinations",
    destinations.map((destination) => ({
      id: destination.id,
      name: destination.config.name,
      provider: destination.config.provider,
      enabled: destination.config.enabled,
      capture_content: destination.config.capture_content,
      capture_reasoning: destination.config.capture_reasoning,
      has_error: !!destination.last_error,
      dropped_events: destination.dropped_events,
    })),
    state,
  );
}
