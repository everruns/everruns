"use client";

import { ScopedCredentialsPanel } from "@/components/credentials/scoped-credentials-panel";

export function AgentCredentialsPanel({ agentId }: { agentId: string }) {
  return <ScopedCredentialsPanel owner={{ scope: "agent", owner_id: agentId, usage: "service" }} />;
}
