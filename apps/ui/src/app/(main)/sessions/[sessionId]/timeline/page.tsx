import { redirect } from "next/navigation";

interface TimelinePageProps {
  params: Promise<{ sessionId: string }>;
}

// Timeline was folded into Trace (knowledge/ui/session-trace.md); keep old links working.
export default async function TimelinePage({ params }: TimelinePageProps) {
  const { sessionId } = await params;
  redirect(`/sessions/${sessionId}/trace`);
}
