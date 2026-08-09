import type { ReactNode } from "react";
import { FeatureFlagPageGate } from "@/components/feature-flag-page-gate";

export default function PluginsLayout({ children }: { children: ReactNode }) {
  return (
    <FeatureFlagPageGate flag="plugins" title="Plugins">
      {children}
    </FeatureFlagPageGate>
  );
}
