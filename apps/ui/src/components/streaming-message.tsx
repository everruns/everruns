"use client";

/**
 * StreamingMessage - Displays streaming text while LLM is generating
 *
 * Shows the accumulated text from output.message.delta events with streaming
 * indicator and markdown rendering via Streamdown.
 */

import { cn } from "@/lib/utils";
import { MessageContent } from "@/components/chat/message-content";
import "./streaming-message.css";

interface StreamingMessageProps {
  text: string;
  className?: string;
}

export function StreamingMessage({ text, className }: StreamingMessageProps) {
  return (
    <div className={cn("relative", className)}>
      <div className="mb-2 inline-flex items-center gap-1.5 text-[10px] uppercase tracking-[0.18em] text-primary/75">
        <span className="relative flex h-1.5 w-1.5">
          <span className="absolute inline-flex h-full w-full animate-ping bg-primary/55" />
          <span className="relative inline-flex h-1.5 w-1.5 bg-primary" />
        </span>
        Generating
      </div>

      <div className="streaming-cursor">
        <MessageContent text={text} isStreaming={true} />
      </div>
    </div>
  );
}
