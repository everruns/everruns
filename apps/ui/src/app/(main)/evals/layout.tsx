import type { ReactNode } from "react";
import { FeatureFlagPageGate } from "@/components/feature-flag-page-gate";

export default function EvalsLayout({ children }: { children: ReactNode }) {
  return (
    <FeatureFlagPageGate flag="evals" title="Evals">
      {children}
    </FeatureFlagPageGate>
  );
}
