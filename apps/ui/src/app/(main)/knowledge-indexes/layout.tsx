import type { ReactNode } from "react";
import { FeatureFlagPageGate } from "@/components/feature-flag-page-gate";

export default function KnowledgeIndexesLayout({ children }: { children: ReactNode }) {
  return (
    <FeatureFlagPageGate flag="knowledge" title="Knowledge indexes">
      {children}
    </FeatureFlagPageGate>
  );
}
