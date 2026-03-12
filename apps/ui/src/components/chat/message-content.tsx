"use client";

/**
 * MessageContent - Renders message text with inline OpenUI blocks.
 *
 * Splits message text on ```openui fenced code blocks and renders
 * alternating StreamdownMessage (markdown) and OpenUIBlock segments.
 * Falls back to pure markdown when no openui blocks are present.
 *
 * @see specs/openui.md
 */

import { splitOpenUIBlocks, hasOpenUIBlocks } from "@/lib/openui-utils";
import { StreamdownMessage } from "@/components/chat/streamdown-message";
import { OpenUIBlock } from "@/components/chat/openui-renderer";

interface MessageContentProps {
  /** Raw message text (may contain ```openui blocks) */
  text: string;
  /** Whether content is actively streaming */
  isStreaming?: boolean;
}

/**
 * Renders message text, splitting OpenUI code blocks into rendered UI
 * and surrounding markdown into Streamdown-rendered text.
 */
export function MessageContent({ text, isStreaming = false }: MessageContentProps) {
  // Fast path: no openui blocks → pure markdown
  if (!hasOpenUIBlocks(text)) {
    return (
      <StreamdownMessage variant="inline" className="text-foreground">
        {text}
      </StreamdownMessage>
    );
  }

  const segments = splitOpenUIBlocks(text);

  return (
    <div className="space-y-2">
      {segments.map((segment, i) =>
        segment.type === "openui" ? (
          <OpenUIBlock key={i} code={segment.content} isStreaming={isStreaming} />
        ) : (
          <StreamdownMessage
            key={i}
            variant="inline"
            className="text-foreground"
            isAnimating={isStreaming}
          >
            {segment.content}
          </StreamdownMessage>
        ),
      )}
    </div>
  );
}
