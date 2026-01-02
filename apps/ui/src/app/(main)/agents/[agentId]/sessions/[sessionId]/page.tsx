"use client";

import { use, useState, useRef, useEffect, useMemo } from "react";
import { useAgent, useSession, useEvents, useSendMessage, useLlmModel } from "@/hooks";
import Link from "next/link";
import { Button } from "@/components/ui/button";
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
import { ArrowLeft, Send, Bot, Loader2, Sparkles, Brain, MessageSquare, Folder, Activity } from "lucide-react";
import type { Controls, ReasoningEffort, FileInfo, ToolCallCompletedData, MessageUserData, MessageAgentData } from "@/lib/api/types";
import { getTextFromContent, isToolCallPart } from "@/lib/api/types";
import { ToolCallCardFromEvent } from "@/components/chat/tool-call-card-from-event";
import { FileBrowser, FileViewer } from "@/components/files";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";

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

  // Fetch events - used for both chat rendering and events tab
  const { data: events, isLoading: eventsLoading } = useEvents(
    agentId,
    sessionId,
    { refetchInterval: shouldPoll ? 1000 : false }
  );

  const [inputValue, setInputValue] = useState("");
  const [reasoningEffort, setReasoningEffort] = useState<ReasoningEffort | "">("");
  const messagesEndRef = useRef<HTMLDivElement>(null);

  // Tab and file viewer state
  const [activeTab, setActiveTab] = useState<"chat" | "files" | "events">("chat");
  const [selectedFile, setSelectedFile] = useState<FileInfo | null>(null);

  // Filter chat-relevant events
  const chatEvents = useMemo(() => {
    if (!events) return [];
    return events.filter(e =>
      e.type === "message.user" ||
      e.type === "message.agent" ||
      e.type === "tool.call_completed"
    );
  }, [events]);

  // Build tool result lookup by tool_call_id
  const toolResultsMap = useMemo(() => {
    const map = new Map<string, ToolCallCompletedData>();
    if (!events) return map;
    for (const event of events) {
      if (event.type === "tool.call_completed") {
        const data = event.data as ToolCallCompletedData;
        map.set(data.tool_call_id, data);
      }
    }
    return map;
  }, [events]);

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

  // Extract text from message event data
  const getMessageText = (data: MessageUserData | MessageAgentData): string => {
    const content = data.message?.content;
    if (!content) return "";
    return getTextFromContent(content);
  };

  // Get tool calls from message event data
  const getToolCalls = (data: MessageAgentData): Array<{ id: string; name: string; arguments: Record<string, unknown> }> => {
    const content = data.message?.content;
    if (!content) return [];
    return content
      .filter(isToolCallPart)
      .map(part => ({ id: part.id, name: part.name, arguments: part.arguments }));
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

        {/* Tabs */}
        <div className="flex gap-1 mt-4">
          <Button
            variant={activeTab === "chat" ? "default" : "ghost"}
            size="sm"
            onClick={() => setActiveTab("chat")}
            className="gap-2"
          >
            <MessageSquare className="h-4 w-4" />
            Chat
          </Button>
          <Button
            variant={activeTab === "files" ? "default" : "ghost"}
            size="sm"
            onClick={() => setActiveTab("files")}
            className="gap-2"
          >
            <Folder className="h-4 w-4" />
            File System
          </Button>
          <Button
            variant={activeTab === "events" ? "default" : "ghost"}
            size="sm"
            onClick={() => setActiveTab("events")}
            className="gap-2"
          >
            <Activity className="h-4 w-4" />
            Events
          </Button>
        </div>
      </div>

      {/* Chat Tab Content */}
      {activeTab === "chat" && (
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
                            <ToolCallCardFromEvent
                              key={tc.id}
                              toolCall={tc}
                              toolResult={toolResult}
                            />
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
        </>
      )}

      {/* Files Tab Content */}
      {activeTab === "files" && (
        <div className="flex-1 flex overflow-hidden">
          <div className="w-1/3 border-r overflow-y-auto">
            <FileBrowser
              agentId={agentId}
              sessionId={sessionId}
              onFileSelect={setSelectedFile}
              selectedPath={selectedFile?.path}
            />
          </div>
          <div className="flex-1 overflow-y-auto">
            {selectedFile && !selectedFile.is_directory ? (
              <FileViewer
                agentId={agentId}
                sessionId={sessionId}
                file={selectedFile}
                onClose={() => setSelectedFile(null)}
              />
            ) : (
              <div className="flex items-center justify-center h-full text-muted-foreground">
                <p>Select a file to view its contents</p>
              </div>
            )}
          </div>
        </div>
      )}

      {/* Events Tab Content */}
      {activeTab === "events" && (
        <div className="flex-1 overflow-y-auto p-4">
          {eventsLoading ? (
            <div className="space-y-2">
              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
            </div>
          ) : events?.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-full text-center text-muted-foreground">
              <Activity className="w-12 h-12 mb-4 opacity-50" />
              <p className="text-lg font-medium">No events yet</p>
              <p className="text-sm">Events will appear here as the session runs</p>
            </div>
          ) : (
            <div className="border rounded-lg">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead className="w-[80px]">Seq</TableHead>
                    <TableHead className="w-[180px]">Type</TableHead>
                    <TableHead className="w-[200px]">Timestamp</TableHead>
                    <TableHead>Data</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {events?.map((event) => (
                    <TableRow key={event.id}>
                      <TableCell className="font-mono text-xs">
                        {event.sequence}
                      </TableCell>
                      <TableCell>
                        <Badge variant="outline" className="font-mono text-xs">
                          {event.type}
                        </Badge>
                      </TableCell>
                      <TableCell className="text-xs text-muted-foreground">
                        {new Date(event.ts).toLocaleString()}
                      </TableCell>
                      <TableCell className="font-mono text-xs max-w-[500px]">
                        <pre className="whitespace-pre-wrap break-all text-xs bg-muted p-2 rounded max-h-[200px] overflow-y-auto">
                          {JSON.stringify(event.data, null, 2)}
                        </pre>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
