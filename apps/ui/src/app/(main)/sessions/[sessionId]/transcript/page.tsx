"use client";

// Transcript is the default human-readable view of a session recording. It
// intentionally reuses the chat transcript so completed replay and live output
// stay identical to Chats, but it never renders a composer or mutation controls.

import { SessionTranscript } from "@/components/session/session-transcript";

export default function TranscriptPage() {
  return <SessionTranscript />;
}
