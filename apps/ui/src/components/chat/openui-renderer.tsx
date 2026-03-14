"use client";

/**
 * OpenUI Renderer - Renders OpenUI Lang code blocks into interactive UI components.
 *
 * Uses @openuidev/react-lang Renderer with the @openuidev/react-ui full library
 * to parse and render OpenUI Lang DSL code into React components (charts, tables,
 * forms, cards, etc.).
 *
 * Wraps the Renderer in an error boundary to catch "Objects are not valid as a
 * React child" errors that occur when the LLM generates malformed OpenUI code
 * (e.g. passing an element where a string prop is expected). The parser doesn't
 * validate prop types, so these objects leak through as raw {type, typeName,
 * props, partial} ElementNode objects.
 *
 * @see specs/openui.md
 * @see https://github.com/thesysdev/openui
 */

import { Renderer } from "@openuidev/react-lang";
import { openuiLibrary } from "@openuidev/react-ui/genui-lib";
import "@openuidev/react-ui/components.css";
import { Component, type ErrorInfo, type ReactNode } from "react";

interface OpenUIBlockProps {
  /** Raw OpenUI Lang code (without the ```openui fences) */
  code: string;
  /** Whether the LLM is still streaming this content */
  isStreaming?: boolean;
}

// ---------------------------------------------------------------------------
// Error boundary – catches render errors from the Renderer (e.g. untyped
// ElementNode objects rendered as React children).
// ---------------------------------------------------------------------------

interface EBProps {
  children: ReactNode;
  fallback: ReactNode;
}
interface EBState {
  hasError: boolean;
}

class OpenUIErrorBoundary extends Component<EBProps, EBState> {
  constructor(props: EBProps) {
    super(props);
    this.state = { hasError: false };
  }

  static getDerivedStateFromError(): EBState {
    return { hasError: true };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.warn("[openui] Render error caught by boundary:", error.message, info.componentStack);
  }

  componentDidUpdate(prevProps: EBProps) {
    // Reset when children change (new code / streaming update)
    if (this.state.hasError && prevProps.children !== this.props.children) {
      this.setState({ hasError: false });
    }
  }

  render() {
    if (this.state.hasError) {
      return this.props.fallback;
    }
    return this.props.children;
  }
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

  const fallback = (
    <pre className="overflow-x-auto p-4 text-xs text-muted-foreground">
      <code>{code}</code>
    </pre>
  );

  return (
    <div className="openui-block my-2 overflow-hidden rounded-lg border border-border bg-card">
      <OpenUIErrorBoundary fallback={fallback}>
        <Renderer response={code} library={openuiLibrary} isStreaming={isStreaming} />
      </OpenUIErrorBoundary>
    </div>
  );
}
