import type { Metadata } from "next";
import { Suspense } from "react";
import { formatPageTitle } from "@/lib/page-title";
import { SandboxFleetTab } from "./fleet-tab";

export const metadata: Metadata = {
  title: formatPageTitle("Sandboxes"),
};

export default function SandboxesFleetPage() {
  return (
    <Suspense>
      <SandboxFleetTab />
    </Suspense>
  );
}
