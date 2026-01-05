"use client";

import { useState, useRef, useEffect } from "react";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Send, Bot, Loader2, Brain } from "lucide-react";
import type { Controls, MessageUserData, MessageAgentData } from "@/lib/api/types";
import { ToolCallCardFromEvent } from "@/components/chat/tool-call-card-from-event";
import { useSessionContext } from "../session-context";

export default function ChatPage() {
  const {
    agentId,
    sessionId,
    session,
    llmModel,
    chatEvents,
    toolResultsMap,
    eventsLoading,
    supportsReasoning,
    reasoningEffort,
    setReasoningEffort,
    getReasoningEffortName,
    defaultEffortName,
    setIsWaitingForResponse,
    sendMessage,
    getMessageText,
    getToolCalls,
  } = useSessionContext();

  const [inputValue, setInputValue] = useState("");
  const messagesEndRef = useRef<HTMLDivElement>(null);

  const reasoningEffortConfig = llmModel?.profile?.reasoning_effort;

  // Auto-scroll to bottom when new events arrive
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [chatEvents]);

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

  return (
    <>
      {/* Messages area */}
      <div className="flex-1 overflow-y-auto p-4 space-y-4">
        {eventsLoading ? (
          <div className="space-y-4">
            <Skeleton className="h-20 w-3/4" />
            <Skeleton className="h-20 w-3/4 ml-auto" />
            <Skeleton className="h-20 w-3/4" />
          </div>
        ) : chatEvents.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-full text-center text-muted-foreground">
            <Bot className="w-12 h-12 mb-4 opacity-50" />
            <p className="text-lg font-medium">No messages yet</p>
            <p className="text-sm">Send a message to start the conversation</p>
          </div>
        ) : (
          chatEvents.map((event) => {
            // Skip tool.call_completed - rendered inline with agent messages
            if (event.type === "tool.call_completed") {
              return null;
            }

            const isUser = event.type === "message.user";
            const data = event.data as MessageUserData | MessageAgentData;
            const textContent = getMessageText(data);
            const toolCalls = isUser ? [] : getToolCalls(data as MessageAgentData);

            return (
              <div key={event.id} className="space-y-2">
                {/* Render text content if present */}
                {textContent && (
                  <div className={`flex ${isUser ? "justify-end" : "justify-start"}`}>
                    {isUser ? (
                      /* User message - dark box, 90% width */
                      <div className="max-w-[90%] bg-gray-500 text-white rounded-lg p-3">
                        <p className="text-sm whitespace-pre-wrap">{textContent}</p>
                      </div>
                    ) : (
                      /* Agent message - darker background with robot icon */
                      <div className="w-full bg-muted/60 rounded-lg p-3">
                        <div className="flex items-start gap-2">
                          <Bot className="w-4 h-4 mt-0.5 flex-shrink-0 text-muted-foreground" />
                          <p className="text-sm whitespace-pre-wrap">{textContent}</p>
                        </div>
                      </div>
                    )}
                  </div>
                )}

                {/* Render tool calls from agent message */}
                {toolCalls.length > 0 && (
                  <div className="pl-[25px] space-y-2">
                    {toolCalls.map((tc) => {
                      const toolResult = toolResultsMap.get(tc.id);
                      return (
                        <ToolCallCardFromEvent key={tc.id} toolCall={tc} toolResult={toolResult} />
                      );
                    })}
                  </div>
                )}
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
            disabled={sendMessage.isPending}
          />
          <Button
            type="submit"
            size="icon"
            className="h-[60px] w-[60px]"
            disabled={!inputValue.trim() || sendMessage.isPending}
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
              onValueChange={(value) => setReasoningEffort(value as typeof reasoningEffort)}
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
      </div>
    </>
  );
}
