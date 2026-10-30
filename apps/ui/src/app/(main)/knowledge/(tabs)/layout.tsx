import { Suspense, type ReactNode } from "react";
import { KnowledgeShell } from "./knowledge-shell";

export default function KnowledgeTabsLayout({ children }: { children: ReactNode }) {
  return (
    <Suspense>
      <KnowledgeShell>{children}</KnowledgeShell>
    </Suspense>
  );
}
