import type { ReactNode } from "react";
import { FeatureFlagPageGate } from "@/components/feature-flag-page-gate";

export default function MemoryLayout({ children }: { children: ReactNode }) {
  return (
    <FeatureFlagPageGate flag="memory" title="Memory">
      {children}
    </FeatureFlagPageGate>
  );
}
