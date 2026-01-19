"use client";

import { cloneElement, isValidElement, type ComponentProps, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { cn } from "@/lib/utils";
import {
  Info,
  Lightbulb,
  AlertTriangle,
  AlertCircle,
  ShieldAlert,
} from "lucide-react";

// GitHub-style alert types
type AlertType = "note" | "tip" | "important" | "warning" | "caution";

interface AlertConfig {
  icon: typeof Info;
  bgClass: string;
  borderClass: string;
  textColor: string;
  titleColor: string;
}

const alertConfigs: Record<AlertType, AlertConfig> = {
  note: {
    icon: Info,
    bgClass: "bg-blue-50 dark:bg-blue-950/30",
    borderClass: "border-blue-400 dark:border-blue-600",
    textColor: "#1e40af",  // blue-800
    titleColor: "#1d4ed8", // blue-700
  },
  tip: {
    icon: Lightbulb,
    bgClass: "bg-green-50 dark:bg-green-950/30",
    borderClass: "border-green-400 dark:border-green-600",
    textColor: "#166534",  // green-800
    titleColor: "#15803d", // green-700
  },
  important: {
    icon: AlertCircle,
    bgClass: "bg-purple-50 dark:bg-purple-950/30",
    borderClass: "border-purple-400 dark:border-purple-600",
    textColor: "#6b21a8",  // purple-800
    titleColor: "#7e22ce", // purple-700
  },
  warning: {
    icon: AlertTriangle,
    bgClass: "bg-yellow-50 dark:bg-yellow-950/30",
    borderClass: "border-yellow-400 dark:border-yellow-600",
    textColor: "#854d0e",  // yellow-800
    titleColor: "#a16207", // yellow-700
  },
  caution: {
    icon: ShieldAlert,
    bgClass: "bg-red-50 dark:bg-red-950/30",
    borderClass: "border-red-400 dark:border-red-600",
    textColor: "#991b1b",  // red-800
    titleColor: "#b91c1c", // red-700
  },
};

// Lightweight markdown styles
const markdownStyles = `
  .markdown-body h1 { font-size: 1.5em; font-weight: 700; margin: 1em 0 0.5em; }
  .markdown-body h2 { font-size: 1.25em; font-weight: 600; margin: 1em 0 0.5em; }
  .markdown-body h3 { font-size: 1.1em; font-weight: 600; margin: 1em 0 0.5em; }
  .markdown-body p { margin: 0.5em 0; }
  .markdown-body ul, .markdown-body ol { margin: 0.5em 0; padding-left: 1.5em; }
  .markdown-body li { margin: 0.25em 0; }
  .markdown-body code { background: var(--color-muted); padding: 0.2em 0.4em; border-radius: 3px; font-size: 0.9em; }
  .markdown-body pre { background: var(--color-muted); padding: 1em; border-radius: 6px; overflow-x: auto; margin: 0.5em 0; }
  .markdown-body pre code { background: none; padding: 0; }
  .markdown-body blockquote { border-left: 3px solid var(--color-border); padding-left: 1em; margin: 0.5em 0; color: var(--color-muted-foreground); }
  .markdown-body hr { border: none; border-top: 1px solid var(--color-border); margin: 1em 0; }
  .markdown-body a { color: var(--color-primary); text-decoration: underline; }
  .markdown-body strong { font-weight: 600; }
  .markdown-body em { font-style: italic; }
  .markdown-body table { border-collapse: collapse; width: 100%; margin: 0.5em 0; }
  .markdown-body th, .markdown-body td { border: 1px solid var(--color-border); padding: 0.5em; text-align: left; }
  .markdown-body th { background: var(--color-muted); font-weight: 600; }
`;

interface NodeWithChildren {
  children?: ReactNode;
}

/**
 * Extract text content from React children recursively
 */
function extractTextContent(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") {
    return String(node);
  }
  if (Array.isArray(node)) {
    return node.map(extractTextContent).join("");
  }
  if (isValidElement<NodeWithChildren>(node) && node.props.children) {
    return extractTextContent(node.props.children);
  }
  return "";
}

/**
 * Remove alert marker from React children, preserving element structure
 */
function removeAlertMarker(children: ReactNode): ReactNode {
  let markerRemoved = false;

  const processNode = (node: ReactNode): ReactNode => {
    if (markerRemoved) return node;

    if (typeof node === "string") {
      const result = node.replace(/^\s*\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]\s*/i, "");
      if (result !== node) {
        markerRemoved = true;
      }
      return result;
    }

    if (Array.isArray(node)) {
      return node.map(processNode);
    }

    if (isValidElement<NodeWithChildren>(node) && node.props.children) {
      const newChildren = processNode(node.props.children);
      if (newChildren !== node.props.children) {
        return cloneElement(node, {}, newChildren);
      }
    }

    return node;
  };

  return processNode(children);
}

/**
 * Parse GitHub-style alert from blockquote content.
 * Format: > [!TYPE]\n> Content
 */
function parseGitHubAlert(children: ReactNode): {
  alertType: AlertType;
  content: ReactNode;
} | null {
  const textContent = extractTextContent(children);

  // Check for GitHub alert syntax: [!TYPE]
  const alertMatch = textContent.match(/^\s*\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]\s*/i);
  if (!alertMatch) {
    return null;
  }

  const alertType = alertMatch[1].toLowerCase() as AlertType;

  return {
    alertType,
    content: removeAlertMarker(children),
  };
}

/**
 * GitHub-style alert component
 */
function GitHubAlert({
  type,
  children,
}: {
  type: AlertType;
  children: ReactNode;
}) {
  const config = alertConfigs[type];
  const Icon = config.icon;
  const title = type.charAt(0).toUpperCase() + type.slice(1);

  return (
    <div
      className={cn(
        "my-4 rounded-md border-l-4 p-4",
        config.bgClass,
        config.borderClass
      )}
    >
      <div
        className="flex items-center gap-2 font-semibold mb-1"
        style={{ color: config.titleColor }}
      >
        <Icon className="h-4 w-4" />
        <span>{title}</span>
      </div>
      <div className="text-sm" style={{ color: config.textColor }}>
        {children}
      </div>
    </div>
  );
}

/**
 * Custom blockquote component that handles GitHub alerts
 */
function Blockquote({ children, ...props }: ComponentProps<"blockquote">) {
  const alertData = parseGitHubAlert(children);

  if (alertData) {
    return <GitHubAlert type={alertData.alertType}>{alertData.content}</GitHubAlert>;
  }

  // Regular blockquote
  return <blockquote {...props}>{children}</blockquote>;
}

interface MarkdownProps {
  content: string;
  className?: string;
  /** Variant for different display contexts */
  variant?: "default" | "compact";
}

/**
 * Markdown renderer with GitHub Flavored Markdown and alert support.
 * Supports GitHub-style alerts: [!NOTE], [!TIP], [!IMPORTANT], [!WARNING], [!CAUTION]
 */
export function Markdown({ content, className, variant = "default" }: MarkdownProps) {
  return (
    <>
      <style>{markdownStyles}</style>
      <div
        className={cn(
          "markdown-body",
          variant === "default" && "bg-muted p-4 rounded-md",
          variant === "compact" && "bg-transparent p-0",
          className
        )}
      >
        <ReactMarkdown
          remarkPlugins={[remarkGfm]}
          components={{
            blockquote: Blockquote,
          }}
        >
          {content}
        </ReactMarkdown>
      </div>
    </>
  );
}

/**
 * Inline markdown for descriptions - supports basic formatting and alerts
 * but without the container styling
 */
export function InlineMarkdown({ content, className }: { content: string; className?: string }) {
  return (
    <>
      <style>{markdownStyles}</style>
      <div className={cn("markdown-body text-sm", className)}>
        <ReactMarkdown
          remarkPlugins={[remarkGfm]}
          components={{
            blockquote: Blockquote,
          }}
        >
          {content}
        </ReactMarkdown>
      </div>
    </>
  );
}
