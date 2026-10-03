import { RouteLoading } from "@/components/layout/route-loading";

// Closer than `(main)/loading` so the session header stays mounted while a
// session page streams in.
export default function SessionLoading() {
  return <RouteLoading />;
}
