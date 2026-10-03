import { Skeleton } from "@/components/ui/skeleton";

/**
 * Placeholder for the content area while a route's server render finishes.
 *
 * Sidebar links disable viewport prefetch, and Next does not prefetch a dynamic
 * route that has no loading boundary. The sidebar's active item follows
 * `usePathname`, which updates only when the navigation commits — so without
 * this boundary the highlight waits for the page's server fetches and paints
 * after the page. The boundary commits the shared shell first; the page streams
 * into this placeholder.
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
