import type { ReactNode } from "react";
import { FeatureFlagPageGate } from "@/components/feature-flag-page-gate";

export default function ReportsLayout({ children }: { children: ReactNode }) {
  return (
    <FeatureFlagPageGate flag="reports" title="Reports">
      {children}
    </FeatureFlagPageGate>
  );
}
