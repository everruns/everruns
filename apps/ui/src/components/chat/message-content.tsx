"use client";

/**
 * MessageContent — renders message text with inline generative-UI blocks.
 *
 * Splits message text on both ```openui and ```a2ui fenced code blocks and
 * renders each segment with its matching renderer. Markdown segments render
 * via StreamdownMessage. Falls back to pure markdown when no UI blocks are
 * present.
 *
 * @see specs/openui.md
 * @see specs/a2ui.md
 */

import { hasA2UIBlocks, splitA2UIBlocks } from "@/lib/a2ui-utils";
import { StreamdownMessage } from "@/components/chat/streamdown-message";
import { OpenUIBlock } from "@/components/chat/openui-renderer";
import { A2UIBlock } from "@/components/chat/a2ui-renderer";

interface MessageContentProps {
  /** Raw message text (may contain ```openui or ```a2ui blocks) */
  text: string;
  /** Whether content is actively streaming */
  isStreaming?: boolean;
}

/**
 * Renders message text, splitting generative-UI code blocks into rendered UI
 * and surrounding markdown into Streamdown-rendered text.
 */
export function MessageContent({ text, isStreaming = false }: MessageContentProps) {
  // Fast path: no generative-UI blocks → pure markdown.
  if (!hasA2UIBlocks(text)) {
    return (
      <StreamdownMessage variant="inline" className="text-foreground">
        {text}
      </StreamdownMessage>
    );
  }

  const segments = splitA2UIBlocks(text);

  return (
    <div className="space-y-2">
      {segments.map((segment, i) => {
        if (segment.type === "openui") {
          return <OpenUIBlock key={i} code={segment.content} isStreaming={isStreaming} />;
        }
        if (segment.type === "a2ui") {
          return <A2UIBlock key={i} code={segment.content} isStreaming={isStreaming} />;
        }
        return (
          <StreamdownMessage
            key={i}
            variant="inline"
            className="text-foreground"
            isAnimating={isStreaming}
          >
            {segment.content}
          </StreamdownMessage>
        );
      })}
    </div>
  );
}
