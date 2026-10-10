"use client";

import { Suspense } from "react";
import { NewAgentFlow } from "@/components/agents/new/new-agent-flow";

export default function NewAgentPage() {
  return (
    <Suspense fallback={null}>
      <NewAgentFlow />
    </Suspense>
  );
}
