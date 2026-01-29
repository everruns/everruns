"use client";

import { useDurableSSE } from "@/hooks";

/**
 * Layout for durable execution pages.
 * Establishes SSE connection for real-time updates across all durable pages.
 */
export default function DurableLayout({ children }: { children: React.ReactNode }) {
  // Connect to global durable SSE stream
  // This single connection updates React Query cache for all durable data
  useDurableSSE();

  return <>{children}</>;
}
