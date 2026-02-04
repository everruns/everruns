"use client";

/**
 * StreamingMessage - Displays streaming text while LLM is generating
 *
 * Shows the accumulated text from output.message.delta events with streaming
 * indicator and markdown rendering via Streamdown.
 */

import { cn } from "@/lib/utils";
import { StreamdownMessage } from "@/components/chat/streamdown-message";

interface StreamingMessageProps {
  text: string;
  className?: string;
}

export function StreamingMessage({ text, className }: StreamingMessageProps) {
  return (
    <div className={cn("relative", className)}>
      {/* Streaming indicator badge */}
      <div className="absolute -top-2 -right-2 flex items-center gap-1 bg-primary/10 text-primary text-xs px-2 py-0.5 rounded-full">
        <span className="relative flex h-2 w-2">
          <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-primary opacity-75" />
          <span className="relative inline-flex rounded-full h-2 w-2 bg-primary" />
        </span>
        Generating
      </div>

      {/* Streaming text content with markdown rendering */}
      <div className="pt-4">
        <StreamdownMessage variant="inline" isAnimating={true}>
          {text}
        </StreamdownMessage>
        {/* Blinking cursor to indicate more content coming */}
        <span className="inline-block w-0.5 h-4 ml-0.5 bg-primary/70 animate-pulse align-text-bottom" />
      </div>
    </div>
  );
}
