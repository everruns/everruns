import { redirect } from "next/navigation";

interface TranscriptPageProps {
  params: Promise<{ sessionId: string }>;
}

// Transcript was folded into Trace (knowledge/ui/session-trace.md); keep old links working.
export default async function TranscriptPage({ params }: TranscriptPageProps) {
  const { sessionId } = await params;
  redirect(`/sessions/${sessionId}/trace`);
}
