import { Skeleton } from "@/components/ui/skeleton";

/**
 * Placeholder for data inside a persistent route layout (such as a session).
 * Do not use at the main route group: that replaces static page chrome during
 * every navigation. Let React retain the current page until the next is ready.
 */
export function RouteLoading() {
  return (
    <div
      role="status"
      aria-busy="true"
      aria-live="polite"
      className="flex flex-col gap-5 p-4 sm:gap-6 sm:p-6"
    >
      <span className="sr-only">Loading</span>
      <Skeleton className="h-4 w-32" />
      <Skeleton className="h-8 w-64" />
      <Skeleton className="h-4 w-full max-w-xl" />
      <div className="grid gap-4 pt-2 sm:grid-cols-2">
        <Skeleton className="h-28 w-full" />
        <Skeleton className="h-28 w-full" />
      </div>
    </div>
  );
}
