"use client";

import { useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { CopyButton } from "@/components/ui/copy-button";
import {
  Bot,
  User,
  Wrench,
  Clock,
  Zap,
  CheckCircle,
  XCircle,
  ChevronRight,
  ChevronDown,
  FileText,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { formatTokens, formatDuration } from "@/lib/formatting";
import type {
  Message,
  ContentPart,
  LlmGenerationData,
  LlmGenerationOutput,
  LlmGenerationMetadata,
  ToolDefinitionSummary,
  TokenUsage,
} from "@/lib/api/types";

// ============================================
// Content Part Renderers
// ============================================

function TextContentPart({ text }: { text: string }) {
  return <p className="text-sm whitespace-pre-wrap">{text}</p>;
}

function ToolCallContentPart({
  toolCall,
}: {
  toolCall: { id: string; name: string; arguments: Record<string, unknown> };
}) {
  return (
    <div className="border rounded-md p-2 bg-muted/50">
      <div className="flex items-center gap-2 mb-1">
        <Wrench className="w-3 h-3 text-muted-foreground" />
        <span className="text-xs font-medium">{toolCall.name}</span>
        <span className="text-xs text-muted-foreground font-mono">({toolCall.id})</span>
      </div>
      <pre className="text-xs bg-background rounded p-2 overflow-x-auto max-h-48 whitespace-pre-wrap break-all">
        {JSON.stringify(toolCall.arguments, null, 2)}
      </pre>
    </div>
  );
}

function ToolResultContentPart({
  toolCallId,
  result,
  error,
}: {
  toolCallId: string;
  result?: unknown;
  error?: string;
}) {
  // Format the result for display
  const formatResult = (value: unknown): string => {
    if (value === undefined || value === null) {
      return "null";
    }
    if (typeof value === "string") {
      // Try to parse and format if it's a JSON string
      try {
        const parsed = JSON.parse(value);
        return JSON.stringify(parsed, null, 2);
      } catch {
        // Not valid JSON, return as-is but handle long strings
        return value;
      }
    }
    return JSON.stringify(value, null, 2);
  };

  const formattedResult = formatResult(result);

  return (
    <div
      className={cn(
        "border rounded-md p-2",
        error
          ? "bg-red-50 border-red-200 dark:bg-red-950/20 dark:border-red-900"
          : "bg-green-50 border-green-200 dark:bg-green-950/20 dark:border-green-900",
      )}
    >
      <div className="flex items-center gap-2 mb-1">
        {error ? (
          <XCircle className="w-3 h-3 text-red-500" />
        ) : (
          <CheckCircle className="w-3 h-3 text-green-500" />
        )}
        <span className="text-xs font-medium">Tool Result</span>
        <span className="text-xs text-muted-foreground font-mono">({toolCallId})</span>
      </div>
      {error ? (
        <p className="text-xs text-red-600 dark:text-red-400">{error}</p>
      ) : (
        <pre className="text-xs bg-background rounded p-2 overflow-x-auto max-h-48 whitespace-pre-wrap break-all">
          {formattedResult}
        </pre>
      )}
    </div>
  );
}

function ImageContentPart({ imageId, filename }: { imageId?: string; filename?: string }) {
  return (
    <div className="border rounded-md p-2 bg-muted/50 flex items-center gap-2">
      <FileText className="w-4 h-4 text-muted-foreground" />
      <span className="text-xs">{filename || imageId || "Image"}</span>
    </div>
  );
}

function ContentPartRenderer({ part }: { part: ContentPart }) {
  switch (part.type) {
    case "text":
      return <TextContentPart text={part.text} />;
    case "tool_call":
      return (
        <ToolCallContentPart
          toolCall={{ id: part.id, name: part.name, arguments: part.arguments }}
        />
      );
    case "tool_result":
      return (
        <ToolResultContentPart
          toolCallId={part.tool_call_id}
          result={part.result}
          error={part.error}
        />
      );
    case "image_file":
      return <ImageContentPart imageId={part.image_id} filename={part.filename} />;
    case "image":
      return <ImageContentPart />;
    default:
      return <pre className="text-xs">{JSON.stringify(part, null, 2)}</pre>;
  }
}

// ============================================
// Message Components
// ============================================

function MessageCard({ message, index }: { message: Message; index: number }) {
  const roleConfig = {
    user: {
      icon: User,
      label: "User",
    },
    agent: {
      icon: Bot,
      label: "Assistant",
    },
    tool_result: {
      icon: Wrench,
      label: "Tool Result",
    },
  };

  // Handle system messages (not in roleConfig)
  const role = message.role as keyof typeof roleConfig;
  const config = roleConfig[role] || {
    icon: FileText,
    label: message.role,
  };
  const Icon = config.icon;

  return (
    <div className="border rounded-lg p-3 bg-background">
      <div className="flex items-center gap-2 mb-2">
        <Icon className="w-4 h-4 text-muted-foreground" />
        <span className="text-sm font-medium">{config.label}</span>
        <span className="text-xs text-muted-foreground">#{index + 1}</span>
        {message.tool_call_id && (
          <span className="text-xs text-muted-foreground font-mono">
            (tool_call_id: {message.tool_call_id})
          </span>
        )}
      </div>
      <div className="space-y-2 pl-6">
        {message.content.map((part, partIndex) => (
          <ContentPartRenderer key={partIndex} part={part} />
        ))}
      </div>
    </div>
  );
}

// ============================================
// Output Section
// ============================================

function OutputSection({ output }: { output: LlmGenerationOutput }) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm flex items-center gap-2">
          <ChevronRight className="w-4 h-4" />
          LLM Output
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        {output.text && (
          <div className="border rounded-lg p-3 bg-muted/30">
            <p className="text-xs font-medium text-muted-foreground mb-1">Text Response</p>
            <p className="text-sm whitespace-pre-wrap">{output.text}</p>
          </div>
        )}
        {output.tool_calls && output.tool_calls.length > 0 && (
          <div>
            <p className="text-xs font-medium text-muted-foreground mb-2">
              Tool Calls ({output.tool_calls.length})
            </p>
            <div className="space-y-2">
              {output.tool_calls.map((toolCall, index) => (
                <ToolCallContentPart key={index} toolCall={toolCall} />
              ))}
            </div>
          </div>
        )}
        {!output.text && (!output.tool_calls || output.tool_calls.length === 0) && (
          <p className="text-sm text-muted-foreground italic">No output</p>
        )}
      </CardContent>
    </Card>
  );
}

// ============================================
// Metadata Section
// ============================================

function MetadataSection({ metadata }: { metadata: LlmGenerationMetadata }) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm">Generation Metadata</CardTitle>
      </CardHeader>
      <CardContent>
        <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
          <div>
            <p className="text-xs text-muted-foreground">Model</p>
            <p className="text-sm font-medium">{metadata.model}</p>
          </div>
          {metadata.provider && (
            <div>
              <p className="text-xs text-muted-foreground">Provider</p>
              <p className="text-sm font-medium">{metadata.provider}</p>
            </div>
          )}
          {metadata.duration_ms !== undefined && (
            <div>
              <p className="text-xs text-muted-foreground flex items-center gap-1">
                <Clock className="w-3 h-3" /> Duration
              </p>
              <p className="text-sm font-medium">{formatDuration(metadata.duration_ms)}</p>
            </div>
          )}
          <div>
            <p className="text-xs text-muted-foreground">Status</p>
            <div className="flex items-center gap-1">
              {metadata.success ? (
                <>
                  <CheckCircle className="w-3 h-3 text-green-500" />
                  <span className="text-sm font-medium text-green-600 dark:text-green-400">
                    Success
                  </span>
                </>
              ) : (
                <>
                  <XCircle className="w-3 h-3 text-red-500" />
                  <span className="text-sm font-medium text-red-600 dark:text-red-400">Failed</span>
                </>
              )}
            </div>
          </div>
        </div>
        {metadata.error && (
          <div className="mt-3 p-2 bg-red-50 dark:bg-red-950/30 border border-red-200 dark:border-red-800 rounded">
            <p className="text-xs font-medium text-red-600 dark:text-red-400">Error</p>
            <p className="text-sm text-red-700 dark:text-red-300">{metadata.error}</p>
          </div>
        )}
        {metadata.usage && <UsageDisplay usage={metadata.usage} />}
      </CardContent>
    </Card>
  );
}

function UsageDisplay({ usage }: { usage: TokenUsage }) {
  const total = usage.input_tokens + usage.output_tokens;

  return (
    <div className="mt-3 p-3 bg-muted/50 rounded-lg">
      <div className="flex items-center gap-2 mb-2">
        <Zap className="w-4 h-4 text-yellow-500" />
        <span className="text-sm font-medium">Token Usage</span>
        <Badge variant="outline" className="ml-auto">
          {formatTokens(total)} total
        </Badge>
      </div>
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3 text-xs">
        <div>
          <p className="text-muted-foreground">Input</p>
          <p className="font-medium">{usage.input_tokens.toLocaleString()}</p>
        </div>
        <div>
          <p className="text-muted-foreground">Output</p>
          <p className="font-medium">{usage.output_tokens.toLocaleString()}</p>
        </div>
        {usage.cache_read_tokens !== undefined && usage.cache_read_tokens > 0 && (
          <div>
            <p className="text-muted-foreground">Cache Read</p>
            <p className="font-medium">{usage.cache_read_tokens.toLocaleString()}</p>
          </div>
        )}
        {usage.cache_creation_tokens !== undefined && usage.cache_creation_tokens > 0 && (
          <div>
            <p className="text-muted-foreground">Cache Created</p>
            <p className="font-medium">{usage.cache_creation_tokens.toLocaleString()}</p>
          </div>
        )}
      </div>
    </div>
  );
}

// ============================================
// Tools Section
// ============================================

function ToolItem({ tool }: { tool: ToolDefinitionSummary }) {
  const [open, setOpen] = useState(false);

  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <CollapsibleTrigger className="flex items-center gap-2 w-full text-left py-1.5 px-2 rounded hover:bg-muted/50 transition-colors">
        {open ? (
          <ChevronDown className="w-3 h-3 text-muted-foreground shrink-0" />
        ) : (
          <ChevronRight className="w-3 h-3 text-muted-foreground shrink-0" />
        )}
        <Wrench className="w-3 h-3 text-muted-foreground shrink-0" />
        <span className="text-xs font-medium">{tool.display_name || tool.name}</span>
        {tool.display_name && tool.display_name !== tool.name && (
          <span className="text-xs text-muted-foreground font-mono">({tool.name})</span>
        )}
      </CollapsibleTrigger>
      <CollapsibleContent>
        <div className="ml-8 pl-2 border-l py-1">
          <p className="text-xs text-muted-foreground whitespace-pre-wrap">{tool.description}</p>
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}

function ToolsSection({ tools }: { tools: ToolDefinitionSummary[] }) {
  const [open, setOpen] = useState(false);

  return (
    <Card>
      <Collapsible open={open} onOpenChange={setOpen}>
        <CardHeader className="pb-2">
          <CollapsibleTrigger className="flex items-center gap-2 w-full text-left">
            {open ? <ChevronDown className="w-4 h-4" /> : <ChevronRight className="w-4 h-4" />}
            <CardTitle className="text-sm">Tools ({tools.length})</CardTitle>
          </CollapsibleTrigger>
        </CardHeader>
        <CollapsibleContent>
          <CardContent className="p-0">
            <ScrollArea className="max-h-[40vh] px-4 pb-4">
              <div className="space-y-0.5">
                {tools.map((tool) => (
                  <ToolItem key={tool.name} tool={tool} />
                ))}
              </div>
            </ScrollArea>
          </CardContent>
        </CollapsibleContent>
      </Collapsible>
    </Card>
  );
}

// ============================================
// Main Component
// ============================================

export interface LlmHistoryViewerProps {
  /** The llm.generation event data */
  data: LlmGenerationData;
  /** Optional event ID for display */
  eventId?: string;
  /** Optional timestamp for display */
  timestamp?: string;
  /** Whether to show in compact mode */
  compact?: boolean;
}

export function LlmHistoryViewer({
  data,
  eventId,
  timestamp,
  compact = false,
}: LlmHistoryViewerProps) {
  const { messages, tools, output, metadata } = data;

  if (compact) {
    return (
      <div className="space-y-3">
        <div className="flex items-center gap-2 text-sm">
          <Badge variant="outline">{metadata.model}</Badge>
          <span className="text-muted-foreground">{messages.length} messages</span>
          {metadata.usage && (
            <span className="text-muted-foreground">
              {formatTokens(metadata.usage.input_tokens + metadata.usage.output_tokens)} tokens
            </span>
          )}
          {metadata.duration_ms !== undefined && (
            <span className="text-muted-foreground">{formatDuration(metadata.duration_ms)}</span>
          )}
        </div>
        <div className="space-y-2">
          {messages.slice(-3).map((message, index) => (
            <MessageCard
              key={message.id || index}
              message={message}
              index={messages.length - 3 + index}
            />
          ))}
          {messages.length > 3 && (
            <p className="text-xs text-muted-foreground text-center">
              +{messages.length - 3} more messages
            </p>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-4">
      {/* Header */}
      {(eventId || timestamp) && (
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Badge variant="outline" className="font-mono">
              llm.generation
            </Badge>
            {eventId && (
              <span className="text-xs text-muted-foreground font-mono flex items-center gap-1">
                {eventId.slice(0, 8)}...
                <CopyButton value={eventId} />
              </span>
            )}
          </div>
          {timestamp && (
            <span className="text-xs text-muted-foreground">
              {new Date(timestamp).toLocaleString()}
            </span>
          )}
        </div>
      )}

      {/* Metadata */}
      <MetadataSection metadata={metadata} />

      {/* Tools */}
      {tools && tools.length > 0 && <ToolsSection tools={tools} />}

      {/* Input Messages - capped height with internal scroll */}
      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-sm">Input Messages ({messages.length})</CardTitle>
        </CardHeader>
        <CardContent className="p-0">
          <ScrollArea className="max-h-[50vh] p-4">
            <div className="space-y-3">
              {messages.map((message, index) => (
                <MessageCard key={message.id || index} message={message} index={index} />
              ))}
            </div>
          </ScrollArea>
        </CardContent>
      </Card>

      {/* Output */}
      <OutputSection output={output} />
    </div>
  );
}
