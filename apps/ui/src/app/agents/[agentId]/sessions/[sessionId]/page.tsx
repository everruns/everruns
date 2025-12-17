"use client";

import { use, useState, useRef, useEffect } from "react";
import { useAgent, useSession, useMessages, useSendMessage } from "@/hooks";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { Textarea } from "@/components/ui/textarea";
import { ArrowLeft, Send, User, Bot, Loader2 } from "lucide-react";
import type { Message } from "@/lib/api/types";

export default function SessionDetailPage({
  params,
}: {
  params: Promise<{ agentId: string; sessionId: string }>;
}) {
  const { agentId, sessionId } = use(params);
  const { data: agent } = useAgent(agentId);

  // Track if we should be polling (after sending a message while session is active)
  const [isPolling, setIsPolling] = useState(false);

  // Poll session status while polling is active
  const { data: session, isLoading: sessionLoading } = useSession(
    agentId,
    sessionId,
    { refetchInterval: isPolling ? 1000 : false }
  );
  const sendMessage = useSendMessage();

  // Determine if session is still processing
  const isActive = session?.status === "running" || session?.status === "pending";

  // Stop polling when session completes
  useEffect(() => {
    if (!isActive && isPolling) {
      setIsPolling(false);
    }
  }, [isActive, isPolling]);

  // Poll for messages while session is active
  const { data: messages, isLoading: messagesLoading } = useMessages(
    agentId,
    sessionId,
    { refetchInterval: isPolling ? 1000 : false }
  );

  const [inputValue, setInputValue] = useState("");
  const messagesEndRef = useRef<HTMLDivElement>(null);

  // Auto-scroll to bottom when new messages arrive
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!inputValue.trim() || sendMessage.isPending) return;

    try {
      await sendMessage.mutateAsync({
        agentId,
        sessionId,
        content: inputValue.trim(),
      });
      setInputValue("");
      // Start polling for the response
      setIsPolling(true);
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

  // Extract message content
  const getMessageContent = (message: Message): string => {
    if (typeof message.content === "object" && message.content !== null) {
      const content = message.content as Record<string, unknown>;
      if (content.text) return String(content.text);
      if (Array.isArray(content)) {
        return content.map((c: { text?: string }) => c.text || "").join("");
      }
    }
    return JSON.stringify(message.content);
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
            {session.status === "completed" && (
              <Badge variant="outline">Completed</Badge>
            )}
            {session.status === "running" && (
              <Badge variant="default">Running</Badge>
            )}
            {session.status === "pending" && (
              <Badge variant="secondary">Pending</Badge>
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
                        <p className="text-xs font-medium opacity-70">
                          {isUser ? "You" : isAssistant ? "Assistant" : message.role}
                        </p>
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
            disabled={sendMessage.isPending || session.status === "completed" || session.status === "failed"}
          />
          <Button
            type="submit"
            size="icon"
            className="h-[60px] w-[60px]"
            disabled={
              !inputValue.trim() ||
              sendMessage.isPending ||
              session.status === "completed" ||
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
        {(session.status === "completed" || session.status === "failed") && (
          <p className="text-xs text-muted-foreground text-center mt-2">
            This session has ended. Start a new session to continue chatting.
          </p>
        )}
      </div>
    </div>
  );
}
