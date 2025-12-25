"use client";

import { useState } from "react";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Wrench, ChevronDown, ChevronRight, CheckCircle2, AlertCircle } from "lucide-react";
import type { Message, ContentPart } from "@/lib/api/types";
import { isToolCallPart, isToolResultPart } from "@/lib/api/types";

interface ToolCallContent {
  id: string;
  name: string;
  arguments: Record<string, unknown>;
}

interface ToolResultContent {
  tool_call_id: string;
  result?: unknown;
  error?: string;
}

// Extract tool call content from ContentPart array
function extractToolCallContent(content: ContentPart[]): ToolCallContent | null {
  for (const part of content) {
    if (isToolCallPart(part)) {
      return {
        id: part.id,
        name: part.name,
        arguments: part.arguments,
      };
    }
  }
  return null;
}

// Extract tool result content from ContentPart array
function extractToolResultContent(content: ContentPart[]): ToolResultContent | null {
  for (const part of content) {
    if (isToolResultPart(part)) {
      return {
        tool_call_id: part.tool_call_id,
        result: part.result,
        error: part.error,
      };
    }
  }
  return null;
}

interface ToolCallCardProps {
  toolCall: Message;
  toolResult?: Message;
}

export function ToolCallCard({ toolCall, toolResult }: ToolCallCardProps) {
  const [isExpanded, setIsExpanded] = useState(false);

  // Handle new ContentPart[] format
  const content = Array.isArray(toolCall.content)
    ? extractToolCallContent(toolCall.content)
    : (toolCall.content as unknown as ToolCallContent);

  const resultContent = toolResult?.content && Array.isArray(toolResult.content)
    ? extractToolResultContent(toolResult.content)
    : (toolResult?.content as unknown as ToolResultContent | undefined);

  const isComplete = !!toolResult;
  const hasError = resultContent?.error !== undefined && resultContent?.error !== null;

  // Handle missing content gracefully
  if (!content) {
    return null;
  }

  return (
    <div className="flex justify-start">
      <Card className="max-w-[90%] bg-muted/50 border-purple-200">
        <CardContent className="p-3">
          <div className="flex items-start gap-2">
            <div className="flex-shrink-0 w-8 h-8 rounded-full bg-purple-100 flex items-center justify-center">
              <Wrench className="w-4 h-4 text-purple-600" />
            </div>
            <div className="flex-1 min-w-0">
              <div className="flex items-center gap-2 flex-wrap">
                <span className="text-xs font-medium text-muted-foreground">Step</span>
                <span className="font-medium text-sm">{content.name}</span>
                {isComplete ? (
                  hasError ? (
                    <Badge variant="destructive" className="text-xs">
                      <AlertCircle className="w-3 h-3 mr-1" />
                      Failed
                    </Badge>
                  ) : (
                    <Badge variant="outline" className="text-xs bg-green-50 text-green-700 border-green-200">
                      <CheckCircle2 className="w-3 h-3 mr-1" />
                      Done
                    </Badge>
                  )
                ) : (
                  <Badge variant="outline" className="text-xs">
                    Running...
                  </Badge>
                )}
              </div>

              {/* Arguments section */}
              {content.arguments && Object.keys(content.arguments).length > 0 && (
                <div className="mt-2">
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-6 px-2 text-xs text-muted-foreground hover:text-foreground"
                    onClick={() => setIsExpanded(!isExpanded)}
                  >
                    {isExpanded ? (
                      <ChevronDown className="w-3 h-3 mr-1" />
                    ) : (
                      <ChevronRight className="w-3 h-3 mr-1" />
                    )}
                    Arguments
                  </Button>
                  {isExpanded && (
                    <pre className="mt-1 p-2 text-xs bg-white rounded border overflow-x-auto max-h-40">
                      {JSON.stringify(content.arguments, null, 2)}
                    </pre>
                  )}
                </div>
              )}

              {/* Result section */}
              {isComplete && resultContent && (
                <div className="mt-2">
                  {hasError ? (
                    <div className="p-2 text-xs bg-red-50 text-red-700 rounded border border-red-200">
                      {resultContent.error}
                    </div>
                  ) : resultContent.result !== undefined ? (
                    <div className="mt-1">
                      <span className="text-xs text-muted-foreground">Result:</span>
                      <pre className="mt-1 p-2 text-xs bg-white rounded border overflow-x-auto max-h-40">
                        {typeof resultContent.result === "string"
                          ? resultContent.result
                          : JSON.stringify(resultContent.result, null, 2)}
                      </pre>
                    </div>
                  ) : null}
                </div>
              )}
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
