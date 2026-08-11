import { redirect } from "next/navigation";

interface LegacySessionChatPageProps {
  params: Promise<{ sessionId: string }>;
}

// Preserve bookmarks from the former read-only session chat route. Mutable
// conversations live under /chats; a session recording's equivalent is Transcript.
export default async function LegacySessionChatPage({ params }: LegacySessionChatPageProps) {
  const { sessionId } = await params;
  redirect(`/sessions/${sessionId}/transcript`);
}
