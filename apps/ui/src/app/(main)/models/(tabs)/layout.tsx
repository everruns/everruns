import { Suspense, type ReactNode } from "react";
import { ModelsShell } from "./models-shell";

// The Models tabs share one shell; each tab's page only claims its path.
export default function ModelsTabsLayout({ children }: { children: ReactNode }) {
  return (
    <Suspense>
      <ModelsShell>{children}</ModelsShell>
    </Suspense>
  );
}
