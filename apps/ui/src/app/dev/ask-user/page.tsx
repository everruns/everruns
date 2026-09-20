"use client";

import { DevPageShell } from "@/app/dev/_components/dev-page-shell";
import { DevChatRuntimeScene } from "@/app/dev/_components/dev-chat-runtime-preview";

export default function AskUserDevPage() {
  return (
    <DevPageShell
      eyebrow="Ask User"
      title="Ask User inline card"
      description="Production transcript cards for structured decisions, deadline nudges, and terminal outcomes."
      widthClassName="max-w-5xl"
    >
      <DevChatRuntimeScene scenario="ask-user" />
    </DevPageShell>
  );
}
