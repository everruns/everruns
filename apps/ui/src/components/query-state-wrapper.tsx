/**
 * Generic wrapper for React Query list pages.
 * Handles loading, error, and empty states so each page
 * only needs to supply its skeleton, empty placeholder, and data render.
 */

import type { ReactNode } from "react";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";

/* ---------- default loading skeleton (card grid) ---------- */

function DefaultLoadingSkeleton({ count = 6 }: { count?: number }) {
  return (
    <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
      {[...Array(count)].map((_, i) => (
        <Card key={i}>
          <CardHeader>
            <Skeleton className="h-6 w-3/4" />
          </CardHeader>
          <CardContent>
            <Skeleton className="h-4 w-full mb-2" />
            <Skeleton className="h-4 w-2/3" />
          </CardContent>
        </Card>
      ))}
    </div>
  );
}

/* ---------- default error ---------- */

function DefaultError({ prefix, message }: { prefix: string; message: string }) {
  return (
    <div className="rounded-lg bg-destructive/10 p-4 text-destructive">
      {prefix}: {message}
    </div>
  );
}

/* ---------- default empty ---------- */

function DefaultEmpty({ message }: { message: string }) {
  return (
    <div className="text-center py-12">
      <p className="text-muted-foreground">{message}</p>
    </div>
  );
}

/* ---------- main component ---------- */

export interface QueryStateWrapperProps<T> {
  /** React Query `isLoading` / `isPending` flag */
  isLoading: boolean;
  /** React Query `error` */
  error: Error | null;
  /** The fetched data (array for list pages) */
  data: T[] | undefined;
  /** Render function called when data is available and non-empty */
  children: (data: T[]) => ReactNode;
  /** Message shown when data is an empty array. Default: "No items found" */
  emptyMessage?: string;
  /** Custom empty state element. Overrides emptyMessage when provided. */
  emptyState?: ReactNode;
  /** Custom loading skeleton element */
  loadingSkeleton?: ReactNode;
  /** Number of skeleton cards when using the default skeleton. Default: 6 */
  skeletonCount?: number;
  /** Prefix for the default error banner (e.g. "Failed to load agents"). Default: "Error" */
  errorMessagePrefix?: string;
  /** Custom error element. Overrides the default error banner when provided. */
  errorState?: ReactNode;
}

export function QueryStateWrapper<T>({
  isLoading,
  error,
  data,
  children,
  emptyMessage = "No items found",
  emptyState,
  loadingSkeleton,
  skeletonCount = 6,
  errorMessagePrefix = "Error",
  errorState,
}: QueryStateWrapperProps<T>) {
  if (error) {
    return errorState ?? <DefaultError prefix={errorMessagePrefix} message={error.message} />;
  }

  if (isLoading) {
    return loadingSkeleton ?? <DefaultLoadingSkeleton count={skeletonCount} />;
  }

  if (!data || data.length === 0) {
    return emptyState ?? <DefaultEmpty message={emptyMessage} />;
  }

  return children(data);
}
