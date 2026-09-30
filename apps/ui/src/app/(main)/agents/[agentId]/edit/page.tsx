import { redirect } from "next/navigation";

// Editing happens in place on the agent page now; keep old links working.
export default async function EditAgentPage({ params }: { params: Promise<{ agentId: string }> }) {
  const { agentId } = await params;
  redirect(`/agents/${agentId}?mode=edit`);
}
