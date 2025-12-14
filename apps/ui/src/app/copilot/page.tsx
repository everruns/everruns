"use client";

import { useState, useCallback, useEffect, useRef } from "react";
import { useAgents, useAgentVersions } from "@/hooks/use-agents";
import { Header } from "@/components/layout/header";
import { AgentSelector } from "@/components/chat/agent-selector";
import { Card, CardContent } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Bot, Plus, Send, User } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { ScrollArea } from "@/components/ui/scroll-area";
import Link from "next/link";

const API_BASE = process.env.NEXT_PUBLIC_API_BASE_URL || "http://localhost:9000";

interface ChatMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
}

// AG-UI event types
interface AgUiEvent {
  type: string;
  threadId?: string;
  runId?: string;
  messageId?: string;
  delta?: string;
  message?: string;
}

export default function CopilotPage() {
  // Agent selection
  const { data: agents = [], isLoading: agentsLoading } = useAgents();
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [selectedVersion, setSelectedVersion] = useState<number | null>(null);
  const { data: versions = [] } = useAgentVersions(selectedAgentId || "");

  // Chat state
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [isStreaming, setIsStreaming] = useState(false);
  const [streamingContent, setStreamingContent] = useState("");
  const [threadId, setThreadId] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  // Set default version when versions load
  useEffect(() => {
    if (versions.length > 0 && !selectedVersion) {
      setSelectedVersion(versions[0].version);
    }
  }, [versions, selectedVersion]);

  // Auto-scroll to bottom
  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages, streamingContent]);

  // Handle agent selection
  const handleAgentChange = useCallback((agentId: string) => {
    setSelectedAgentId(agentId);
    setSelectedVersion(null);
    setMessages([]);
    setThreadId(null);
  }, []);

  // Handle sending a message
  const handleSend = useCallback(async () => {
    if (!input.trim() || !selectedAgentId || !selectedVersion || isStreaming) return;

    const userMessage: ChatMessage = {
      id: crypto.randomUUID(),
      role: "user",
      content: input.trim(),
    };

    setMessages((prev) => [...prev, userMessage]);
    setInput("");
    setIsStreaming(true);
    setStreamingContent("");

    try {
      // Build the AG-UI endpoint URL
      const params = new URLSearchParams({
        agent_id: selectedAgentId,
        agent_version: selectedVersion.toString(),
      });
      if (threadId) {
        params.set("thread_id", threadId);
      }

      const url = `${API_BASE}/v1/ag-ui?${params}`;

      // Send request with messages
      const response = await fetch(url, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          messages: messages.concat(userMessage).map((m) => ({
            role: m.role,
            content: m.content,
          })),
        }),
      });

      if (!response.ok) {
        throw new Error(`HTTP error: ${response.status}`);
      }

      // Get thread ID from response headers
      const responseThreadId = response.headers.get("X-Thread-Id");
      if (responseThreadId) {
        setThreadId(responseThreadId);
      }

      // Parse SSE stream
      const reader = response.body?.getReader();
      if (!reader) throw new Error("No response body");

      const decoder = new TextDecoder();
      let buffer = "";
      let currentContent = "";
      let currentMessageId = "";

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });

        // Process complete SSE events
        const lines = buffer.split("\n");
        buffer = lines.pop() || ""; // Keep incomplete line in buffer

        for (const line of lines) {
          if (line.startsWith("data: ")) {
            const jsonStr = line.slice(6);
            try {
              const event: AgUiEvent = JSON.parse(jsonStr);
              console.log("AG-UI Event:", event);

              switch (event.type) {
                case "RunStarted":
                  if (event.threadId) {
                    setThreadId(event.threadId);
                  }
                  break;

                case "TextMessageStart":
                  currentMessageId = event.messageId || crypto.randomUUID();
                  currentContent = "";
                  break;

                case "TextMessageContent":
                  if (event.delta) {
                    currentContent += event.delta;
                    setStreamingContent(currentContent);
                  }
                  break;

                case "TextMessageEnd":
                  if (currentContent) {
                    const assistantMessage: ChatMessage = {
                      id: currentMessageId,
                      role: "assistant",
                      content: currentContent,
                    };
                    setMessages((prev) => [...prev, assistantMessage]);
                    setStreamingContent("");
                  }
                  break;

                case "RunFinished":
                case "RunError":
                  setIsStreaming(false);
                  break;
              }
            } catch {
              // Ignore parse errors for keepalive comments
            }
          }
        }
      }

      setIsStreaming(false);
    } catch (error) {
      console.error("Failed to send message:", error);
      setIsStreaming(false);
    }
  }, [input, selectedAgentId, selectedVersion, messages, threadId, isStreaming]);

  const selectedAgent = agents.find((a) => a.id === selectedAgentId);
  const canChat = selectedAgentId && selectedVersion && versions.length > 0;

  return (
    <>
      <Header title="Chat (AG-UI)" />
      <div className="flex flex-col h-[calc(100vh-64px)]">
        {agentsLoading ? (
          <div className="p-6">
            <Skeleton className="h-12 w-full" />
          </div>
        ) : agents.length === 0 ? (
          <div className="flex-1 flex items-center justify-center">
            <Card className="max-w-md">
              <CardContent className="pt-6 text-center">
                <Bot className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
                <h3 className="text-lg font-medium mb-2">No agents available</h3>
                <p className="text-muted-foreground mb-4">
                  Create an agent with at least one version to start chatting.
                </p>
                <Link href="/agents/new">
                  <Button>
                    <Plus className="h-4 w-4 mr-2" />
                    Create Agent
                  </Button>
                </Link>
              </CardContent>
            </Card>
          </div>
        ) : (
          <>
            {/* Agent Selector */}
            <AgentSelector
              agents={agents.filter((a) => a.status === "active")}
              versions={versions}
              selectedAgentId={selectedAgentId}
              selectedVersion={selectedVersion}
              onAgentChange={handleAgentChange}
              onVersionChange={setSelectedVersion}
              disabled={isStreaming}
            />

            {/* Chat Area */}
            {!canChat ? (
              <div className="flex-1 flex items-center justify-center">
                <div className="text-center">
                  <Bot className="h-16 w-16 mx-auto text-muted-foreground mb-4" />
                  <h3 className="text-lg font-medium mb-2">Select an agent to start</h3>
                  <p className="text-muted-foreground">
                    {selectedAgentId && versions.length === 0
                      ? "This agent has no versions. Create a version first."
                      : "Choose an agent from the dropdown above"}
                  </p>
                </div>
              </div>
            ) : (
              <div className="flex-1 flex flex-col">
                {/* Messages */}
                <ScrollArea className="flex-1 p-4" ref={scrollRef}>
                  <div className="space-y-4 max-w-3xl mx-auto">
                    {messages.map((message) => (
                      <div
                        key={message.id}
                        className={`flex gap-3 ${
                          message.role === "user" ? "justify-end" : "justify-start"
                        }`}
                      >
                        {message.role === "assistant" && (
                          <div className="w-8 h-8 rounded-full bg-primary/10 flex items-center justify-center">
                            <Bot className="h-4 w-4 text-primary" />
                          </div>
                        )}
                        <div
                          className={`rounded-lg px-4 py-2 max-w-[80%] ${
                            message.role === "user"
                              ? "bg-primary text-primary-foreground"
                              : "bg-muted"
                          }`}
                        >
                          <p className="whitespace-pre-wrap">{message.content}</p>
                        </div>
                        {message.role === "user" && (
                          <div className="w-8 h-8 rounded-full bg-primary flex items-center justify-center">
                            <User className="h-4 w-4 text-primary-foreground" />
                          </div>
                        )}
                      </div>
                    ))}

                    {/* Streaming message */}
                    {streamingContent && (
                      <div className="flex gap-3 justify-start">
                        <div className="w-8 h-8 rounded-full bg-primary/10 flex items-center justify-center">
                          <Bot className="h-4 w-4 text-primary animate-pulse" />
                        </div>
                        <div className="rounded-lg px-4 py-2 max-w-[80%] bg-muted">
                          <p className="whitespace-pre-wrap">{streamingContent}</p>
                        </div>
                      </div>
                    )}

                    {/* Loading indicator */}
                    {isStreaming && !streamingContent && (
                      <div className="flex gap-3 justify-start">
                        <div className="w-8 h-8 rounded-full bg-primary/10 flex items-center justify-center">
                          <Bot className="h-4 w-4 text-primary animate-pulse" />
                        </div>
                        <div className="rounded-lg px-4 py-2 bg-muted">
                          <p className="text-muted-foreground">Thinking...</p>
                        </div>
                      </div>
                    )}
                  </div>
                </ScrollArea>

                {/* Input */}
                <div className="border-t p-4">
                  <div className="max-w-3xl mx-auto flex gap-2">
                    <Textarea
                      value={input}
                      onChange={(e) => setInput(e.target.value)}
                      placeholder={`Message ${selectedAgent?.name || "agent"}...`}
                      disabled={isStreaming}
                      className="min-h-[44px] max-h-32 resize-none"
                      onKeyDown={(e) => {
                        if (e.key === "Enter" && !e.shiftKey) {
                          e.preventDefault();
                          handleSend();
                        }
                      }}
                    />
                    <Button
                      onClick={handleSend}
                      disabled={!input.trim() || isStreaming}
                      size="icon"
                      className="h-11 w-11"
                    >
                      <Send className="h-4 w-4" />
                    </Button>
                  </div>
                </div>
              </div>
            )}
          </>
        )}
      </div>
    </>
  );
}
