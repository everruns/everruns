import { redirect } from "next/navigation";

// Sessions are a tab on the agent page now; keep old links working.
export default async function AgentSessionsPage({
  params,
}: {
  params: Promise<{ agentId: string }>;
}) {
  const { agentId } = await params;
  redirect(`/agents/${agentId}?tab=sessions`);
}
