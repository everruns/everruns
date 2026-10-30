"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { DevPageShell } from "@/app/dev/_components/dev-page-shell";
import { DevChatRuntimeScene } from "@/app/dev/_components/dev-chat-runtime-preview";

export default function AskUserDevPage() {
  const ui = useUiTranslations();
  return (
    <DevPageShell
      eyebrow="Ask User"
      title={ui("Ask User inline card")}
      description={ui(
        "Production transcript cards for structured decisions, deadline nudges, and terminal outcomes.",
      )}
      widthClassName="max-w-5xl"
    >
      <DevChatRuntimeScene scenario="ask-user" />
    </DevPageShell>
  );
}
