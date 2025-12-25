"use client";

import { use, useState, useRef, useEffect } from "react";
import { useAgent, useSession, useEvents, useSendMessage, useLlmModel } from "@/hooks";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ArrowLeft, Send, User, Bot, Loader2, Sparkles, Brain } from "lucide-react";
import type { Message, Controls, ReasoningEffort } from "@/lib/api/types";
import { getTextFromContent, isToolCallPart } from "@/lib/api/types";
import { ToolCallCard } from "@/components/chat/tool-call-card";

export default function SessionDetailPage({
  params,
}: {
  params: Promise<{ agentId: string; sessionId: string }>;
}) {
  const { agentId, sessionId } = use(params);
  const { data: agent } = useAgent(agentId);

  // Track if user has sent a message and is waiting for response
  const [isWaitingForResponse, setIsWaitingForResponse] = useState(false);

  // First fetch session without polling to get initial status
  const { data: session, isLoading: sessionLoading } = useSession(
    agentId,
    sessionId
  );
  const sendMessage = useSendMessage();

  // Fetch LLM model info if session has a model_id
  const { data: llmModel } = useLlmModel(session?.model_id ?? "");

  // Determine if session is still processing
  const isActive = session?.status === "running" || session?.status === "pending";

  // Derive whether we should poll - only when waiting AND session is active
  const shouldPoll = isWaitingForResponse && isActive;

  // Re-fetch session with polling when shouldPoll changes
  // This uses the same query key, so it just updates the polling interval
  useSession(agentId, sessionId, {
    refetchInterval: shouldPoll ? 1000 : false,
  });

  // Poll for messages (from events) while waiting and session is active
  // Uses events endpoint and transforms to Message format for display
  const { data: messages, isLoading: messagesLoading } = useEvents(
    agentId,
    sessionId,
    { refetchInterval: shouldPoll ? 1000 : false }
  );

  const [inputValue, setInputValue] = useState("");
  const [reasoningEffort, setReasoningEffort] = useState<ReasoningEffort | "">("");
  const messagesEndRef = useRef<HTMLDivElement>(null);

  // Check if the model supports reasoning effort
  const supportsReasoning = llmModel?.profile?.reasoning && llmModel?.profile?.reasoning_effort;
  const reasoningEffortConfig = llmModel?.profile?.reasoning_effort;

  // Get display name for a reasoning effort value
  const getReasoningEffortName = (value: string): string => {
    const effort = reasoningEffortConfig?.values.find(e => e.value === value);
    return effort?.name ?? value;
  };

  // Get the default effort display name
  const defaultEffortName = reasoningEffortConfig?.default
    ? getReasoningEffortName(reasoningEffortConfig.default)
    : "Medium";

  // Auto-scroll to bottom when new messages arrive
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!inputValue.trim() || sendMessage.isPending) return;

    // Build controls with reasoning effort if selected
    const controls: Controls | undefined = reasoningEffort
      ? { reasoning: { effort: reasoningEffort } }
      : undefined;

    try {
      await sendMessage.mutateAsync({
        agentId,
        sessionId,
        content: inputValue.trim(),
        controls,
      });
      setInputValue("");
      // Start polling for the response
      setIsWaitingForResponse(true);
    } catch (error) {
      console.error("Failed to send message:", error);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSubmit(e);
    }
  };

  // Extract message content - handles new ContentPart[] format
  const getMessageContent = (message: Message): string => {
    if (Array.isArray(message.content)) {
      return getTextFromContent(message.content);
    }
    return JSON.stringify(message.content);
  };

  // Build a map of tool_call_id to tool_result messages
  const toolResultsMap = new Map<string, Message>();
  messages?.forEach((msg) => {
    if (msg.role === "tool_result" && msg.tool_call_id) {
      toolResultsMap.set(msg.tool_call_id, msg);
    }
  });

  // Get tool call ID from message content - handles new ContentPart[] format
  const getToolCallId = (message: Message): string | null => {
    if (Array.isArray(message.content)) {
      for (const part of message.content) {
        if (isToolCallPart(part)) {
          return part.id;
        }
      }
    }
    return null;
  };

  // Check if assistant message has embedded tool_calls (for LLM context only, not UI display)
  const hasEmbeddedToolCalls = (message: Message): boolean => {
    if (message.role !== "assistant") return false;
    if (Array.isArray(message.content)) {
      return message.content.some(isToolCallPart);
    }
    return false;
  };

  if (sessionLoading) {
    return (
      <div className="container mx-auto p-6">
        <Skeleton className="h-8 w-1/3 mb-4" />
        <Skeleton className="h-4 w-2/3 mb-8" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (!session) {
    return (
      <div className="container mx-auto p-6">
        <div className="text-red-500">Session not found</div>
        <Link
          href={`/agents/${agentId}`}
          className="text-blue-500 hover:underline"
        >
          Back to agent
        </Link>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-[calc(100vh-4rem)]">
      {/* Header */}
      <div className="border-b p-4">
        <Link
          href={`/agents/${agentId}`}
          className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-2"
        >
          <ArrowLeft className="w-4 h-4 mr-2" />
          Back to {agent?.name || "Agent"}
        </Link>

        <div className="flex items-center justify-between">
          <div>
            <h1 className="text-xl font-bold">
              {session.title || `Session ${session.id.slice(0, 8)}`}
            </h1>
            <p className="text-sm text-muted-foreground">
              Started {new Date(session.created_at).toLocaleString()}
            </p>
          </div>
          <div className="flex items-center gap-2">
            {llmModel && (
              <Badge variant="outline" className="gap-1">
                <Sparkles className="w-3 h-3" />
                {llmModel.display_name}
              </Badge>
            )}
            {session.status === "running" && (
              <Badge variant="default">Processing...</Badge>
            )}
            {session.status === "pending" && (
              <Badge variant="secondary">Ready</Badge>
            )}
            {session.status === "failed" && (
              <Badge variant="destructive">Failed</Badge>
            )}
          </div>
        </div>
      </div>

      {/* Messages area */}
      <div className="flex-1 overflow-y-auto p-4 space-y-4">
        {messagesLoading ? (
          <div className="space-y-4">
            <Skeleton className="h-20 w-3/4" />
            <Skeleton className="h-20 w-3/4 ml-auto" />
            <Skeleton className="h-20 w-3/4" />
          </div>
        ) : messages?.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-center text-muted-foreground">
            <Bot className="w-12 h-12 mb-4 opacity-50" />
            <p className="text-lg font-medium">No messages yet</p>
            <p className="text-sm">Send a message to start the conversation</p>
          </div>
        ) : (
          messages?.map((message) => {
            const isUser = message.role === "user";
            const isAssistant = message.role === "assistant";
            const isToolCall = message.role === "tool_call";
            const isToolResult = message.role === "tool_result";

            // Skip tool_result messages - they're rendered with their tool_call
            if (isToolResult) {
              return null;
            }

            // Skip assistant messages with embedded tool_calls - they're for LLM context only
            // The actual tool calls are rendered as separate ToolCallCard components
            if (hasEmbeddedToolCalls(message)) {
              return null;
            }

            // Render tool calls with their results
            if (isToolCall) {
              const toolCallId = getToolCallId(message);
              const toolResult = toolCallId ? toolResultsMap.get(toolCallId) : undefined;
              return (
                <ToolCallCard
                  key={message.id}
                  toolCall={message}
                  toolResult={toolResult}
                />
              );
            }

            // Extract metadata for assistant messages
            const messageModel = isAssistant ? (message.metadata?.model as string | undefined) : undefined;
            const messageReasoningEffort = isAssistant ? (message.metadata?.reasoning_effort as string | undefined) : undefined;

            // Render user and assistant messages
            return (
              <div
                key={message.id}
                className={`flex ${isUser ? "justify-end" : "justify-start"}`}
              >
                <Card
                  className={`max-w-[80%] ${
                    isUser
                      ? "bg-primary text-primary-foreground"
                      : "bg-muted"
                  }`}
                >
                  <CardContent className="p-3">
                    <div className="flex items-start gap-2">
                      {!isUser && (
                        <Bot className="w-5 h-5 mt-0.5 flex-shrink-0" />
                      )}
                      <div className="space-y-1">
                        <div className="flex items-center gap-2">
                          <p className="text-xs font-medium opacity-70">
                            {isUser ? "You" : isAssistant ? "Assistant" : message.role}
                          </p>
                          {/* Show model and reasoning info for assistant messages */}
                          {isAssistant && (messageModel || messageReasoningEffort) && (
                            <div className="flex items-center gap-1">
                              {messageModel && (
                                <Badge variant="outline" className="text-[10px] px-1 py-0 h-4 gap-0.5">
                                  <Sparkles className="w-2.5 h-2.5" />
                                  {messageModel}
                                </Badge>
                              )}
                              {messageReasoningEffort && (
                                <Badge variant="outline" className="text-[10px] px-1 py-0 h-4 gap-0.5">
                                  <Brain className="w-2.5 h-2.5" />
                                  {messageReasoningEffort}
                                </Badge>
                              )}
                            </div>
                          )}
                        </div>
                        <p className="text-sm whitespace-pre-wrap">
                          {getMessageContent(message)}
                        </p>
                      </div>
                      {isUser && (
                        <User className="w-5 h-5 mt-0.5 flex-shrink-0" />
                      )}
                    </div>
                  </CardContent>
                </Card>
              </div>
            );
          })
        )}
        <div ref={messagesEndRef} />
      </div>

      {/* Input area */}
      <div className="border-t p-4">
        <form onSubmit={handleSubmit} className="flex gap-2">
          <Textarea
            value={inputValue}
            onChange={(e) => setInputValue(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Type a message... (Enter to send, Shift+Enter for newline)"
            className="flex-1 min-h-[60px] max-h-[200px] resize-none"
            disabled={sendMessage.isPending || session.status === "failed"}
          />
          <Button
            type="submit"
            size="icon"
            className="h-[60px] w-[60px]"
            disabled={
              !inputValue.trim() ||
              sendMessage.isPending ||
              session.status === "failed"
            }
          >
            {sendMessage.isPending ? (
              <Loader2 className="h-5 w-5 animate-spin" />
            ) : (
              <Send className="h-5 w-5" />
            )}
          </Button>
        </form>
        {/* Reasoning effort selector - only shown when model supports it */}
        {supportsReasoning && reasoningEffortConfig && (
          <div className="flex items-center gap-2 mt-2">
            <Brain className="h-4 w-4 text-muted-foreground" />
            <span className="text-sm text-muted-foreground">Reasoning:</span>
            <Select
              value={reasoningEffort}
              onValueChange={(value) => setReasoningEffort(value as ReasoningEffort | "")}
            >
              <SelectTrigger size="sm" className="w-[180px]">
                <SelectValue>
                  {reasoningEffort
                    ? getReasoningEffortName(reasoningEffort)
                    : `Default (${defaultEffortName})`}
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="">{`Default (${defaultEffortName})`}</SelectItem>
                {reasoningEffortConfig.values.map((effort) => (
                  <SelectItem key={effort.value} value={effort.value}>
                    {effort.name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        )}
        {session.status === "failed" && (
          <p className="text-xs text-muted-foreground text-center mt-2">
            This session has failed. Start a new session to continue chatting.
          </p>
        )}
      </div>
    </div>
  );
}
