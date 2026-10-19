import { Suspense, type ReactNode } from "react";
import { WebSearchShell } from "./web-search-shell";

// The Web search tabs share one shell; each tab's page only claims its path.
export default function WebSearchTabsLayout({ children }: { children: ReactNode }) {
  return (
    <Suspense>
      <WebSearchShell>{children}</WebSearchShell>
    </Suspense>
  );
}
