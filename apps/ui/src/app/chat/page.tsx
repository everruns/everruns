"use client";

import { useState, useCallback, useEffect } from "react";
import { useAgents, useAgentVersions } from "@/hooks/use-agents";
import { useCreateThread, useMessages, useCreateMessage } from "@/hooks/use-threads";
import { useCreateRun, useRun } from "@/hooks/use-runs";
import { useSSEEvents, aggregateTextMessages, aggregateToolCalls } from "@/hooks/use-sse-events";
import { Header } from "@/components/layout/header";
import { AgentSelector } from "@/components/chat/agent-selector";
import { ChatMessages } from "@/components/chat/chat-messages";
import { ChatInput } from "@/components/chat/chat-input";
import { Card, CardContent } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Bot, Plus, ExternalLink } from "lucide-react";
import Link from "next/link";
import type { AgUiEvent } from "@/lib/api/types";

export default function ChatPage() {
  // Agent selection
  const { data: agents = [], isLoading: agentsLoading } = useAgents();
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null);
  const [selectedVersion, setSelectedVersion] = useState<number | null>(null);
  const { data: versions = [] } = useAgentVersions(selectedAgentId || "");

  // Thread state
  const [threadId, setThreadId] = useState<string | null>(null);
  const { data: messages = [] } = useMessages(threadId || "");
  const createThread = useCreateThread();
  const createMessage = useCreateMessage(threadId || "");

  // Run state
  const [currentRunId, setCurrentRunId] = useState<string | null>(null);
  const { data: currentRun } = useRun(currentRunId || "");
  const createRun = useCreateRun();

  // SSE events for current run
  const { events, isConnected } = useSSEEvents({
    runId: currentRunId || "",
    enabled: !!currentRunId && currentRun?.status === "running",
  });

  // Derived state
  const streamingMessages = aggregateTextMessages(events);
  const streamingToolCalls = aggregateToolCalls(events);
  const isStreaming = !!currentRunId && currentRun?.status === "running";
  const isWaiting = createThread.isPending || createMessage.isPending || createRun.isPending;

  // Set default version when versions load
  useEffect(() => {
    if (versions.length > 0 && !selectedVersion) {
      setSelectedVersion(versions[0].version);
    }
  }, [versions, selectedVersion]);

  // Clear current run when it finishes
  useEffect(() => {
    if (currentRun && (currentRun.status === "completed" || currentRun.status === "failed")) {
      // Keep the run ID for a moment to show final state, then clear
      const timer = setTimeout(() => {
        setCurrentRunId(null);
      }, 500);
      return () => clearTimeout(timer);
    }
  }, [currentRun?.status]);

  // Handle agent selection
  const handleAgentChange = useCallback((agentId: string) => {
    setSelectedAgentId(agentId);
    setSelectedVersion(null);
    // Reset conversation when agent changes
    setThreadId(null);
    setCurrentRunId(null);
  }, []);

  // Handle sending a message
  const handleSendMessage = useCallback(
    async (content: string) => {
      if (!selectedAgentId || !selectedVersion) return;

      try {
        // Create thread if needed
        let tid = threadId;
        if (!tid) {
          const thread = await createThread.mutateAsync();
          tid = thread.id;
          setThreadId(tid);
        }

        // Add user message
        await createMessage.mutateAsync({
          role: "user",
          content,
        });

        // Create run
        const run = await createRun.mutateAsync({
          agent_id: selectedAgentId,
          agent_version: selectedVersion,
          thread_id: tid,
        });

        setCurrentRunId(run.id);
      } catch (error) {
        console.error("Failed to send message:", error);
      }
    },
    [selectedAgentId, selectedVersion, threadId, createThread, createMessage, createRun]
  );

  // Handle new conversation
  const handleNewConversation = useCallback(() => {
    setThreadId(null);
    setCurrentRunId(null);
  }, []);

  const selectedAgent = agents.find((a) => a.id === selectedAgentId);
  const canChat = selectedAgentId && selectedVersion && versions.length > 0;

  return (
    <>
      <Header
        title="Chat"
        action={
          threadId && (
            <Button variant="outline" onClick={handleNewConversation}>
              <Plus className="h-4 w-4 mr-2" />
              New Conversation
            </Button>
          )
        }
      />
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
                  {selectedAgentId && versions.length === 0 && (
                    <Link href={`/agents/${selectedAgentId}`}>
                      <Button variant="link">
                        Create agent version
                        <ExternalLink className="h-4 w-4 ml-1" />
                      </Button>
                    </Link>
                  )}
                </div>
              </div>
            ) : (
              <>
                {/* Thread Info Bar */}
                {threadId && (
                  <div className="px-4 py-2 bg-muted/50 border-b flex items-center gap-2 text-sm">
                    <span className="text-muted-foreground">Thread:</span>
                    <Link
                      href={`/threads/${threadId}`}
                      className="font-mono text-primary hover:underline"
                    >
                      {threadId.slice(0, 8)}...
                    </Link>
                    {currentRunId && (
                      <>
                        <span className="text-muted-foreground">|</span>
                        <span className="text-muted-foreground">Run:</span>
                        <Link
                          href={`/runs/${currentRunId}`}
                          className="font-mono text-primary hover:underline"
                        >
                          {currentRunId.slice(0, 8)}...
                        </Link>
                        {isStreaming && (
                          <Badge variant="outline" className="bg-blue-100 text-blue-800 animate-pulse">
                            Streaming
                          </Badge>
                        )}
                      </>
                    )}
                  </div>
                )}

                {/* Messages */}
                <ChatMessages
                  messages={messages}
                  streamingMessages={streamingMessages}
                  streamingToolCalls={streamingToolCalls}
                  isStreaming={isStreaming}
                />

                {/* Input */}
                <ChatInput
                  onSend={handleSendMessage}
                  disabled={isStreaming || isWaiting}
                  placeholder={
                    isStreaming
                      ? "Waiting for response..."
                      : `Message ${selectedAgent?.name || "agent"}...`
                  }
                />
              </>
            )}
          </>
        )}
      </div>
    </>
  );
}
