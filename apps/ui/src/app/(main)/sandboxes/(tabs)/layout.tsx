import { Suspense, type ReactNode } from "react";
import { SandboxesShell } from "./sandboxes-shell";

export default function SandboxesTabsLayout({ children }: { children: ReactNode }) {
  return (
    <Suspense>
      <SandboxesShell>{children}</SandboxesShell>
    </Suspense>
  );
}
