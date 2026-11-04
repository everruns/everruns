import { RouteLoading } from "@/components/layout/route-loading";

// Keep the session header mounted while a session page streams in.
export default function SessionLoading() {
  return <RouteLoading />;
}
