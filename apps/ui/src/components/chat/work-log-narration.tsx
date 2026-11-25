"use client";

import { InlineMarkdownMessage } from "@/components/markdown/markdown-message";

interface WorkLogNarrationProps {
  children: string;
}

/** Renders human-readable work-log narration with the shared safe Markdown semantics. */
export function WorkLogNarration({ children }: WorkLogNarrationProps) {
  return (
    <InlineMarkdownMessage
      className="markdown-work-log min-w-0 flex-1 whitespace-pre-wrap text-[15px] leading-6 text-muted-foreground"
      skipHtml
    >
      {children}
    </InlineMarkdownMessage>
  );
}
