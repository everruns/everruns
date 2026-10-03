import type { ReactNode } from "react";
import { FeatureFlagPageGate } from "@/components/feature-flag-page-gate";

export default function PlaygroundLayout({ children }: { children: ReactNode }) {
  return (
    <FeatureFlagPageGate flag="playground" title="Playground">
      {children}
    </FeatureFlagPageGate>
  );
}
