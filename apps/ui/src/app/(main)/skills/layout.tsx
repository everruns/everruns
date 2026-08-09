import type { ReactNode } from "react";
import { FeatureFlagPageGate } from "@/components/feature-flag-page-gate";

export default function SkillsLayout({ children }: { children: ReactNode }) {
  return (
    <FeatureFlagPageGate flag="skills" title="Skills">
      {children}
    </FeatureFlagPageGate>
  );
}
