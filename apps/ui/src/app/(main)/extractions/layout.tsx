import type { ReactNode } from "react";
import { FeatureFlagPageGate } from "@/components/feature-flag-page-gate";

export default function ExtractionsLayout({ children }: { children: ReactNode }) {
  return (
    <FeatureFlagPageGate flag="extractions" title="Extractions">
      {children}
    </FeatureFlagPageGate>
  );
}
