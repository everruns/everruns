import { redirect } from "next/navigation";

interface SessionPageProps {
  params: Promise<{ agentId: string; sessionId: string }>;
}

export default async function SessionPage({ params }: SessionPageProps) {
  const { agentId, sessionId } = await params;
  redirect(`/agents/${agentId}/sessions/${sessionId}/chat`);
}
