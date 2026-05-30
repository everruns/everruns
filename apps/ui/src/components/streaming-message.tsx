"use client";

/**
 * StreamingMessage - Displays streaming text while LLM is generating
 *
 * Shows the accumulated text from output.message.delta events with streaming
 * indicator and markdown rendering via Streamdown.
 */

import { useEffect, useMemo, useState } from "react";
import { cn } from "@/lib/utils";
import { MessageContent } from "@/components/chat/message-content";
import "./streaming-message.css";

const FRAME_MS = 16;

type GraphemeSegment = { segment: string };
type SegmenterConstructor = new (
  locales?: string | string[],
  options?: { granularity: "grapheme" },
) => { segment(input: string): Iterable<GraphemeSegment> };

function splitCharacters(text: string): string[] {
  const Segmenter = (Intl as typeof Intl & { Segmenter?: SegmenterConstructor }).Segmenter;
  if (Segmenter) {
    const segmenter = new Segmenter(undefined, { granularity: "grapheme" });
    return Array.from(segmenter.segment(text), ({ segment }) => segment);
  }
  return Array.from(text);
}

function countSharedPrefix(a: string[], b: string[]): number {
  const max = Math.min(a.length, b.length);
  for (let i = 0; i < max; i++) {
    if (a[i] !== b[i]) return i;
  }
  return max;
}

function charactersPerFrame(remaining: number): number {
  if (remaining > 240) return 10;
  if (remaining > 120) return 6;
  if (remaining > 48) return 3;
  return 1;
}

function useSmoothedText(targetText: string): string {
  const targetCharacters = useMemo(() => splitCharacters(targetText), [targetText]);
  const [visibleCharacters, setVisibleCharacters] = useState<string[]>(() =>
    targetCharacters.slice(0, Math.min(1, targetCharacters.length)),
  );

  useEffect(() => {
    setVisibleCharacters((current) => {
      if (targetCharacters.length === 0) return [];

      const sharedPrefix = countSharedPrefix(current, targetCharacters);
      if (sharedPrefix === current.length) return current;

      return targetCharacters.slice(0, Math.max(sharedPrefix, 1));
    });
  }, [targetCharacters]);

  useEffect(() => {
    if (visibleCharacters.length >= targetCharacters.length) return;

    const timeout = window.setTimeout(() => {
      setVisibleCharacters((current) => {
        const sharedPrefix = countSharedPrefix(current, targetCharacters);
        const remaining = targetCharacters.length - sharedPrefix;
        const nextLength = sharedPrefix + charactersPerFrame(remaining);
        return targetCharacters.slice(0, Math.min(nextLength, targetCharacters.length));
      });
    }, FRAME_MS);

    return () => window.clearTimeout(timeout);
  }, [targetCharacters, visibleCharacters]);

  return visibleCharacters.join("");
}

interface StreamingMessageProps {
  text: string;
  className?: string;
}

export function StreamingMessage({ text, className }: StreamingMessageProps) {
  const visibleText = useSmoothedText(text);

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
        <MessageContent text={visibleText} isStreaming={true} />
      </div>
    </div>
  );
}
