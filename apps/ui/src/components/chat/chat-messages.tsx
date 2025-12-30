"use client";

import { useEffect, useRef } from "react";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Avatar, AvatarFallback } from "@/components/ui/avatar";
import { Badge } from "@/components/ui/badge";
import { Separator } from "@/components/ui/separator";
import { Bot, User, Wrench, Loader2 } from "lucide-react";
import { cn } from "@/lib/utils";
import type { Message } from "@/lib/api/types";
import { getTextFromContent } from "@/lib/api/types";
import type { AggregatedMessage, AggregatedToolCall } from "@/hooks/use-sse-events";
import { TodoListRenderer, isWriteTodosTool } from "./todo-list-renderer";

interface ChatMessagesProps {
  messages: Message[];
  streamingMessages: AggregatedMessage[];
  streamingToolCalls: AggregatedToolCall[];
  isStreaming: boolean;
}

function MessageBubble({
  role,
  content,
  isStreaming,
}: {
  role: string;
  content: string;
  isStreaming?: boolean;
}) {
  const isUser = role === "user";

  return (
    <div className={cn("flex gap-3", isUser && "flex-row-reverse")}>
      <Avatar className="h-8 w-8 shrink-0">
        <AvatarFallback className={cn(isUser ? "bg-primary" : "bg-muted")}>
          {isUser ? <User className="h-4 w-4" /> : <Bot className="h-4 w-4" />}
        </AvatarFallback>
      </Avatar>
      <div
        className={cn(
          "rounded-lg px-4 py-2 max-w-[80%]",
          isUser ? "bg-primary text-primary-foreground" : "bg-muted"
        )}
      >
        <p className="whitespace-pre-wrap">{content}</p>
        {isStreaming && (
          <span className="inline-block w-2 h-4 bg-current opacity-75 animate-pulse ml-0.5" />
        )}
      </div>
    </div>
  );
}

function ToolCallBubble({ toolCall }: { toolCall: AggregatedToolCall }) {
  // Special rendering for write_todos tool
  if (isWriteTodosTool(toolCall.name)) {
    return (
      <div className="flex gap-3">
        <Avatar className="h-8 w-8 shrink-0">
          <AvatarFallback className="bg-purple-100">
            <Wrench className="h-4 w-4 text-purple-600" />
          </AvatarFallback>
        </Avatar>
        <div className="border rounded-lg p-3 max-w-[80%] bg-purple-50">
          <TodoListRenderer
            arguments={toolCall.arguments}
            result={toolCall.result}
            isExecuting={!toolCall.isComplete}
            error={toolCall.error}
          />
        </div>
      </div>
    );
  }

  return (
    <div className="flex gap-3">
      <Avatar className="h-8 w-8 shrink-0">
        <AvatarFallback className="bg-purple-100">
          <Wrench className="h-4 w-4 text-purple-600" />
        </AvatarFallback>
      </Avatar>
      <div className="border rounded-lg p-3 max-w-[80%] bg-purple-50">
        <div className="flex items-center gap-2 mb-2">
          <span className="font-medium text-sm">{toolCall.name}</span>
          {toolCall.isComplete ? (
            toolCall.error ? (
              <Badge variant="destructive" className="text-xs">
                Failed
              </Badge>
            ) : (
              <Badge variant="outline" className="bg-green-100 text-green-800 text-xs">
                Done
              </Badge>
            )
          ) : (
            <Badge variant="outline" className="text-xs">
              <Loader2 className="h-3 w-3 mr-1 animate-spin" />
              Running
            </Badge>
          )}
        </div>
        <pre className="text-xs bg-white p-2 rounded overflow-x-auto">
          {JSON.stringify(toolCall.arguments, null, 2)}
        </pre>
        {toolCall.isComplete && toolCall.result !== undefined && (
          <>
            <Separator className="my-2" />
            <pre className="text-xs bg-white p-2 rounded overflow-x-auto max-h-32">
              {JSON.stringify(toolCall.result, null, 2)}
            </pre>
          </>
        )}
        {toolCall.error && (
          <>
            <Separator className="my-2" />
            <p className="text-sm text-destructive">{toolCall.error}</p>
          </>
        )}
      </div>
    </div>
  );
}

export function ChatMessages({
  messages,
  streamingMessages,
  streamingToolCalls,
  isStreaming,
}: ChatMessagesProps) {
  const scrollRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to bottom when new content arrives
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages, streamingMessages, streamingToolCalls]);

  return (
    <ScrollArea className="flex-1 p-4" ref={scrollRef}>
      <div className="space-y-4">
        {/* Existing messages from history */}
        {messages.map((msg) => (
          <MessageBubble
            key={msg.id}
            role={msg.role}
            content={Array.isArray(msg.content)
              ? getTextFromContent(msg.content)
              : JSON.stringify(msg.content)}
          />
        ))}

        {/* Streaming tool calls */}
        {streamingToolCalls.map((tc) => (
          <ToolCallBubble key={tc.id} toolCall={tc} />
        ))}

        {/* Streaming messages */}
        {streamingMessages.map((msg) => (
          <MessageBubble
            key={msg.id}
            role={msg.role}
            content={msg.content}
            isStreaming={!msg.isComplete}
          />
        ))}

        {/* Loading indicator when waiting for response */}
        {isStreaming && streamingMessages.length === 0 && streamingToolCalls.length === 0 && (
          <div className="flex gap-3">
            <Avatar className="h-8 w-8 shrink-0">
              <AvatarFallback className="bg-muted">
                <Bot className="h-4 w-4" />
              </AvatarFallback>
            </Avatar>
            <div className="bg-muted rounded-lg px-4 py-2">
              <div className="flex gap-1">
                <div className="w-2 h-2 bg-current rounded-full animate-bounce" />
                <div className="w-2 h-2 bg-current rounded-full animate-bounce delay-75" />
                <div className="w-2 h-2 bg-current rounded-full animate-bounce delay-150" />
              </div>
            </div>
          </div>
        )}
      </div>
    </ScrollArea>
  );
}
