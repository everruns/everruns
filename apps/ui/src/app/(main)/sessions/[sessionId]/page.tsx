import { redirect } from "next/navigation";

interface SessionPageProps {
  params: Promise<{ sessionId: string }>;
}

export default async function SessionPage({ params }: SessionPageProps) {
  const { sessionId } = await params;
  redirect(`/sessions/${sessionId}/chat`);
}
