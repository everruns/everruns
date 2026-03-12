"use client";

/**
 * OpenUI Renderer - Renders OpenUI Lang code blocks into interactive UI components.
 *
 * Uses @openuidev/react-lang Renderer with the @openuidev/react-ui chat library
 * to parse and render OpenUI Lang DSL code into React components (charts, tables,
 * forms, cards, etc.).
 *
 * @see specs/openui.md
 * @see https://github.com/thesysdev/openui
 */

import { Renderer } from "@openuidev/react-lang";
import { openuiChatLibrary } from "@openuidev/react-ui/genui-lib";
import "@openuidev/react-ui/components.css";

interface OpenUIBlockProps {
  /** Raw OpenUI Lang code (without the ```openui fences) */
  code: string;
  /** Whether the LLM is still streaming this content */
  isStreaming?: boolean;
}

/**
 * Renders a single OpenUI Lang code block into interactive UI components.
 *
 * The Renderer component from @openuidev/react-lang handles:
 * - Parsing the OpenUI Lang DSL
 * - Resolving forward references (hoisting)
 * - Progressive rendering during streaming
 * - Error boundaries for malformed output
 */
export function OpenUIBlock({ code, isStreaming = false }: OpenUIBlockProps) {
  if (!code.trim()) return null;

  return (
    <div className="openui-block my-2 overflow-hidden rounded-lg border border-border bg-card">
      <Renderer response={code} library={openuiChatLibrary} isStreaming={isStreaming} />
    </div>
  );
}
