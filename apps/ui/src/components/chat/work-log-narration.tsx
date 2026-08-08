"use client";

import { InlineStreamdownMessage } from "@/components/chat/streamdown-message";

interface WorkLogNarrationProps {
  children: string;
}

/** Renders human-readable work-log narration with the shared safe Markdown semantics. */
export function WorkLogNarration({ children }: WorkLogNarrationProps) {
  return (
    <InlineStreamdownMessage
      className="streamdown-work-log min-w-0 flex-1 whitespace-pre-wrap text-[15px] leading-6 text-muted-foreground"
      skipHtml
    >
      {children}
    </InlineStreamdownMessage>
  );
}
