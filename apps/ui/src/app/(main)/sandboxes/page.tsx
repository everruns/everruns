import type { Metadata } from "next";
import { Suspense } from "react";
import { formatPageTitle } from "@/lib/page-title";
import SandboxesPageClient from "./sandboxes-page-client";

export const metadata: Metadata = {
  title: formatPageTitle("Sandboxes"),
};

export default function SandboxesPage() {
  return (
    <Suspense>
      <SandboxesPageClient />
    </Suspense>
  );
}
