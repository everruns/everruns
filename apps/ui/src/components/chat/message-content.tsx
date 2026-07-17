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
import type { TextAnnotation } from "@/lib/api/schema-types";
import {
  type CitationSource,
  MessageSources,
  injectCitationMarkers,
  numberCitations,
} from "@/components/chat/message-citations";

interface MessageContentProps {
  /** Raw message text (may contain ```openui or ```a2ui blocks) */
  text: string;
  /** Whether content is actively streaming */
  isStreaming?: boolean;
  /** Claim-level citations attached to the message text (see specs/citations.md). */
  annotations?: TextAnnotation[];
}

/**
 * Renders message text, splitting generative-UI code blocks into rendered UI
 * and surrounding markdown into Streamdown-rendered text. When the message
 * carries citation annotations, inline chips and a source strip are rendered.
 */
export function MessageContent({ text, isStreaming = false, annotations }: MessageContentProps) {
  // Citation path: inline chips + source strip. Only in the fast (no
  // generative-UI blocks) path — chip markers are char-offset based and a2ui
  // splitting would shift offsets. Skipped while streaming (offsets apply to
  // the finalized message only).
  if (annotations && annotations.length > 0 && !isStreaming && !hasA2UIBlocks(text)) {
    const { sources } = numberCitations(annotations);
    const citations = new Map<number, CitationSource>(sources.map((s) => [s.n, s]));
    return (
      <div>
        <StreamdownMessage variant="inline" className="text-foreground" citations={citations}>
          {injectCitationMarkers(text, annotations)}
        </StreamdownMessage>
        <MessageSources sources={sources} />
      </div>
    );
  }

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
