"use client";

import { useState, useRef, useEffect, useMemo, useCallback, useLayoutEffect } from "react";
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
import {
  Send,
  Bot,
  Loader2,
  Brain,
  ImagePlus,
  StopCircle,
  CalendarClock,
  ArrowDown,
} from "lucide-react";
import type {
  ActCompletedData,
  ActStartedData,
  Controls,
  InputMessageData,
  OutputMessageCompletedData,
  ContentPart,
  CommandDescriptor,
  ToolCallRequestedData,
  ToolCallSummary,
  ToolCompletedData,
  ToolStartedData,
} from "@/lib/api/types";
import { isImageFilePart } from "@/lib/api/types";
import { MessageInfoIcon } from "@/components/chat/message-info-icon";
import { ImageAttachments, MessageImage } from "@/components/chat/image-attachments";
import {
  CommandAutocomplete,
  shouldShowCommandAutocomplete,
} from "@/components/chat/command-autocomplete";
import { ThinkingIndicator } from "@/components/thinking-indicator";
import { StreamingMessage } from "@/components/streaming-message";
import { MessageContent } from "@/components/chat/message-content";
import { ToolActivityGroup } from "@/components/chat/tool-activity-group";
import { SetupConnectionToolCall } from "@/components/chat/setup-connection-tool-call";
import {
  ToolActivityTimelineGroup,
  type TimelineToolRow,
} from "@/components/chat/tool-activity-timeline-group";
import {
  formatWorkedDuration,
  getCompletedTurnDurationsByEvent,
} from "@/components/chat/turn-delimiter";
import { useSessionContext } from "@/app/(main)/sessions/[sessionId]/session-context";
import { useLlmModels, useImageAttachments, useSessionCommands } from "@/hooks";
import { sendUserMessageWithImages } from "@/lib/api/messages";
import { useMutation } from "@tanstack/react-query";
import { ALLOWED_IMAGE_TYPES } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { chatSurfaceStyles } from "@/components/chat/chat-surface";

/** Expandable compaction divider — click to see cascade details. */
function CompactionDivider({ data }: { data: import("@/lib/api/types").ContextCompactedData }) {
  const [expanded, setExpanded] = useState(false);
  const saved = data.messages_before - data.messages_after;

  return (
    <div className="space-y-1">
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className="flex w-full cursor-pointer items-center gap-4 py-2 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground sm:text-sm"
      >
        <div className="h-px flex-1 bg-border" />
        <span className="whitespace-nowrap">
          Context compacted · {data.messages_before} → {data.messages_after} messages
          {data.strategy_used !== "none" && ` · ${data.strategy_used}`}
        </span>
        <span className="text-[10px]">{expanded ? "▲" : "▼"}</span>
        <div className="h-px flex-1 bg-border" />
      </button>
      {expanded && (
        <div className="mx-auto max-w-md rounded-md border bg-muted/50 px-4 py-2 text-xs text-muted-foreground">
          <div className="space-y-1">
            <div>
              <span className="font-medium">Saved:</span> {saved} messages in {data.duration_ms}ms
            </div>
            {data.steps.length > 0 && (
              <div>
                <span className="font-medium">Cascade steps:</span>
                <ol className="mt-1 list-inside list-decimal space-y-0.5">
                  {data.steps.map((step, i) => (
                    <li key={i}>
                      {step.strategy} → {step.messages_after} messages ({step.duration_ms}ms)
                    </li>
                  ))}
                </ol>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

export function ChatPanel() {
  const {
    agentId,
    events,
    sessionId,
    llmModel,
    chatEvents,
    toolResultsMap,
    eventsLoading,
    isActive,
    reasoningEffort,
    setReasoningEffort,
    setIsWaitingForResponse,
    isThinking,
    streamingText,
    streamingIteration,
    sendMessage,
    cancelCurrentTurn,
    hasMoreEvents,
    loadingOlderEvents,
    loadOlderEvents,
    getMessageText,
    getToolCalls,
  } = useSessionContext();

  const { data: llmModels = [] } = useLlmModels();

  const [inputValue, setInputValue] = useState("");
  const [selectedModelId, setSelectedModelId] = useState("");
  const [isDraggingOver, setIsDraggingOver] = useState(false);
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const isNearBottomRef = useRef(true);
  const [hasNewMessages, setHasNewMessages] = useState(false);
  const prevChatEventsLengthRef = useRef(0);
  const initialScrollDoneRef = useRef(false);

  // Track scroll height before prepending older events for position preservation
  const prevScrollHeightRef = useRef<number | null>(null);
  const hasUserSelectedModel = useRef(false);
  const modelSelectionStorageKey = useMemo(
    () => `everruns:chat:model-selection:${agentId}:${sessionId}`,
    [agentId, sessionId],
  );

  // Image attachments management
  const {
    pendingImages,
    allUploaded,
    uploadedImageIds,
    addFiles,
    removeImage,
    clearImages,
    hasImages,
    isUploading,
  } = useImageAttachments({ sessionId });

  // Commands autocomplete
  const { data: commandsData } = useSessionCommands(sessionId);
  const commands = commandsData?.commands ?? [];
  const hasCommands = commands.length > 0;
  const [showCommands, setShowCommands] = useState(false);
  const inputPlaceholder = hasCommands
    ? "Type a message or / for commands... (Enter to send)"
    : "Type a message... (Enter to send)";

  const clientRequestedToolCallIds = useMemo(() => {
    const ids = new Set<string>();
    for (const event of chatEvents) {
      if (event.type !== "tool.call_requested") continue;
      const data = event.data as ToolCallRequestedData;
      for (const toolCall of data.tool_calls ?? []) {
        ids.add(toolCall.id);
      }
    }
    return ids;
  }, [chatEvents]);

  const turnDurationByEventId = useMemo(
    () => getCompletedTurnDurationsByEvent(events ?? []),
    [events],
  );

  const hasNarratedActEvents = useMemo(
    () => chatEvents.some((event) => event.type === "act.started"),
    [chatEvents],
  );

  const actGroupsByStartEventId = useMemo(() => {
    type GroupState = {
      startEventId: string;
      headline: string;
      completedHeadline?: string;
      rows: TimelineToolRow[];
    };

    const groupsByExecId = new Map<string, GroupState>();

    const ensureRow = (
      group: GroupState,
      id: string,
      fallbackLabel: string,
      state: TimelineToolRow["state"],
    ) => {
      const existing = group.rows.find((row) => row.id === id);
      if (existing) {
        existing.label = existing.label || fallbackLabel;
        existing.state = state;
        return existing;
      }

      const row: TimelineToolRow = { id, label: fallbackLabel, state };
      group.rows.push(row);
      return row;
    };

    for (const event of chatEvents) {
      const execId = event.context?.exec_id;
      if (!execId) continue;

      if (event.type === "act.started") {
        const data = event.data as ActStartedData;
        groupsByExecId.set(execId, {
          startEventId: event.id,
          headline: data.headline ?? "Working",
          rows: (data.tool_calls ?? []).map((toolCall: ToolCallSummary) => ({
            id: toolCall.id,
            label: toolCall.narration ?? toolCall.display_name ?? toolCall.name,
            state: "running",
          })),
        });
        continue;
      }

      const group = groupsByExecId.get(execId);
      if (!group) continue;

      if (event.type === "tool.started") {
        const data = event.data as ToolStartedData;
        const row = ensureRow(
          group,
          data.tool_call.id,
          data.narration ?? data.display_name ?? data.tool_call.name,
          "running",
        );
        row.label = data.narration ?? row.label;
        continue;
      }

      if (event.type === "tool.completed") {
        const data = event.data as ToolCompletedData;
        const row = ensureRow(
          group,
          data.tool_call_id,
          data.narration ?? data.display_name ?? data.tool_name ?? "Tool call",
          data.success ? "completed" : "error",
        );
        row.label = data.narration ?? row.label;
        row.result = data;
        row.state = data.success ? "completed" : "error";
        continue;
      }

      if (event.type === "act.completed") {
        const data = event.data as ActCompletedData;
        group.completedHeadline = data.headline;
      }
    }

    return new Map(
      Array.from(groupsByExecId.values()).map((group) => [group.startEventId, group] as const),
    );
  }, [chatEvents]);

  const handleCommandSelect = useCallback(
    (cmd: CommandDescriptor) => {
      setShowCommands(false);
      if (cmd.source === "system") {
        // System commands: send as slash message directly
        setInputValue(`/${cmd.name}`);
        // Submit immediately
        const controls: Controls | undefined = selectedModelId
          ? { model_id: selectedModelId }
          : undefined;
        sendMessage.mutateAsync({
          sessionId,
          content: `/${cmd.name}`,
          controls,
        });
        setInputValue("");
        setIsWaitingForResponse(true);
        textareaRef.current?.focus();
      } else {
        // Skill commands: inject as message to trigger prompt expansion
        setInputValue(`/${cmd.name} `);
        textareaRef.current?.focus();
      }
    },
    [sessionId, selectedModelId, sendMessage, setIsWaitingForResponse],
  );

  useEffect(() => {
    if (typeof window === "undefined") return;
    const storedSelection = window.localStorage.getItem(modelSelectionStorageKey);
    if (storedSelection !== null && !hasUserSelectedModel.current) {
      setSelectedModelId(storedSelection);
    }
  }, [modelSelectionStorageKey]);

  // Derived guard: if commands disappear while dropdown is open, close it inline
  if (!hasCommands && showCommands) {
    setShowCommands(false);
  }

  const selectedModel = useMemo(
    () => llmModels.find((model) => model.id === selectedModelId),
    [llmModels, selectedModelId],
  );

  const activeModel = selectedModel ?? llmModel;
  const reasoningEffortConfig = activeModel?.profile?.reasoning_effort;
  const supportsReasoning = !!(
    activeModel?.profile?.reasoning && activeModel?.profile?.reasoning_effort
  );

  const getReasoningEffortName = (value: string): string => {
    const effort = reasoningEffortConfig?.values.find((item) => item.value === value);
    return effort?.name ?? value;
  };

  const defaultEffortName = reasoningEffortConfig?.default
    ? getReasoningEffortName(reasoningEffortConfig.default)
    : "Medium";
  const modelTriggerLabel = selectedModel?.display_name ?? "Default";
  const defaultModelOptionLabel = llmModel?.display_name
    ? `Default (${llmModel.display_name})`
    : "Default";

  useEffect(() => {
    if (!supportsReasoning) {
      setReasoningEffort("");
      return;
    }

    if (
      reasoningEffortConfig &&
      reasoningEffort &&
      !reasoningEffortConfig.values.some((effort) => effort.value === reasoningEffort)
    ) {
      setReasoningEffort("");
    }
  }, [supportsReasoning, reasoningEffortConfig, reasoningEffort, setReasoningEffort]);

  // Track whether user is near the bottom of the scroll container
  const NEAR_BOTTOM_THRESHOLD = 100;

  const checkIsNearBottom = useCallback(() => {
    const container = scrollContainerRef.current;
    if (!container) return true;
    const { scrollTop, scrollHeight, clientHeight } = container;
    return scrollHeight - scrollTop - clientHeight <= NEAR_BOTTOM_THRESHOLD;
  }, []);

  const scrollToBottom = useCallback((behavior: ScrollBehavior = "smooth") => {
    messagesEndRef.current?.scrollIntoView({ behavior });
  }, []);

  // Listen for scroll events to track near-bottom state and trigger older event loading
  useEffect(() => {
    const container = scrollContainerRef.current;
    if (!container) return;

    const handleScroll = () => {
      const nearBottom = checkIsNearBottom();
      isNearBottomRef.current = nearBottom;
      // Clear "new messages" indicator when user scrolls back to bottom
      if (nearBottom) {
        setHasNewMessages(false);
      }
    };

    container.addEventListener("scroll", handleScroll, { passive: true });
    return () => container.removeEventListener("scroll", handleScroll);
  }, [checkIsNearBottom]);

  // Scroll to bottom immediately on initial load (once events are ready)
  useEffect(() => {
    if (!eventsLoading && chatEvents.length > 0 && !initialScrollDoneRef.current) {
      initialScrollDoneRef.current = true;
      // Use requestAnimationFrame to ensure DOM is painted
      requestAnimationFrame(() => {
        scrollToBottom("instant");
      });
    }
  }, [eventsLoading, chatEvents.length, scrollToBottom]);

  // Reset initial scroll flag when session changes
  useEffect(() => {
    initialScrollDoneRef.current = false;
    isNearBottomRef.current = true;
    setHasNewMessages(false);
    prevChatEventsLengthRef.current = 0;
  }, [sessionId]);

  // Auto-scroll on new events or streaming updates (only when near bottom)
  // Skip if older events were prepended (prevScrollHeightRef is set)
  useEffect(() => {
    if (!initialScrollDoneRef.current) return;
    if (prevScrollHeightRef.current !== null) return;

    const hasNewChatEvents = chatEvents.length > prevChatEventsLengthRef.current;
    prevChatEventsLengthRef.current = chatEvents.length;

    if (isNearBottomRef.current) {
      scrollToBottom("smooth");
    } else if (hasNewChatEvents) {
      setHasNewMessages(true);
    }
  }, [chatEvents, streamingText, isThinking, scrollToBottom]);

  // Scroll position preservation: when older events are prepended,
  // adjust scrollTop so the viewport stays in the same visual position
  useLayoutEffect(() => {
    const container = scrollContainerRef.current;
    const prevHeight = prevScrollHeightRef.current;
    if (container && prevHeight !== null) {
      const newHeight = container.scrollHeight;
      container.scrollTop += newHeight - prevHeight;
      prevScrollHeightRef.current = null;
    }
  }, [chatEvents]);

  // Scroll-up detection: trigger loading older events
  const handleScroll = useCallback(() => {
    const container = scrollContainerRef.current;
    if (!container || !hasMoreEvents || loadingOlderEvents) return;

    // Trigger when user scrolls within 200px of the top
    if (container.scrollTop < 200) {
      // Save scroll height before prepend for position preservation
      prevScrollHeightRef.current = container.scrollHeight;
      loadOlderEvents();
    }
  }, [hasMoreEvents, loadingOlderEvents, loadOlderEvents]);

  // Auto-focus message input when session loads
  useEffect(() => {
    if (!eventsLoading) {
      textareaRef.current?.focus();
    }
  }, [eventsLoading]);

  // Mutation for sending message with images
  const sendMessageWithImages = useMutation({
    mutationFn: async ({
      text,
      images,
      controls,
    }: {
      text: string;
      images: Array<{ imageId: string; filename?: string }>;
      controls?: Controls;
    }) => {
      return sendUserMessageWithImages(sessionId, text, images, controls);
    },
  });

  // Check if can submit (has content and all images uploaded)
  const hasContent = inputValue.trim() || hasImages;
  const canSubmit =
    hasContent && allUploaded && !sendMessage.isPending && !sendMessageWithImages.isPending;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;

    // Build controls with reasoning effort if selected
    const controls: Controls | undefined =
      selectedModelId || reasoningEffort
        ? {
            ...(selectedModelId && { model_id: selectedModelId }),
            ...(reasoningEffort &&
              supportsReasoning && {
                reasoning: { effort: reasoningEffort },
              }),
          }
        : undefined;

    try {
      if (hasImages) {
        // Send with images
        await sendMessageWithImages.mutateAsync({
          text: inputValue.trim(),
          images: uploadedImageIds,
          controls,
        });
        clearImages();
      } else {
        // Use the regular sendMessage for text-only (has optimistic UI)
        await sendMessage.mutateAsync({
          sessionId,
          content: inputValue.trim(),
          controls,
        });
      }

      if (typeof window !== "undefined") {
        window.localStorage.setItem(modelSelectionStorageKey, selectedModelId);
      }
      setInputValue("");
      // Start polling for the response
      setIsWaitingForResponse(true);
      // Refocus textarea after send
      textareaRef.current?.focus();
    } catch (error) {
      console.error("Failed to send message:", error);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Let command autocomplete handle keyboard when visible
    if (showCommands) return;
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSubmit(e);
    }
  };

  // Handle file input change
  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = e.target.files;
    if (files && files.length > 0) {
      addFiles(Array.from(files));
    }
    // Reset input so same file can be selected again
    e.target.value = "";
  };

  // Extract image files from message content
  const getMessageImages = (
    content: ContentPart[],
  ): Array<{ image_id: string; filename?: string }> => {
    return content.filter(isImageFilePart).map((part) => ({
      image_id: part.image_id,
      filename: part.filename,
    }));
  };

  const renderTurnDivider = (eventId: string) => {
    const durationMs = turnDurationByEventId.get(eventId);

    if (durationMs == null) {
      return null;
    }

    return (
      <div className="flex items-center gap-4 pt-3 text-xs font-medium text-muted-foreground sm:text-sm">
        <div className="h-px flex-1 bg-border" />
        <span className="whitespace-nowrap">{`Worked for ${formatWorkedDuration(durationMs)}`}</span>
        <div className="h-px flex-1 bg-border" />
      </div>
    );
  };

  return (
    <>
      <div
        ref={scrollContainerRef}
        onScroll={handleScroll}
        className={cn(
          "relative flex-1 overflow-y-auto bg-background bg-brand-dots px-4 py-5 sm:px-6",
          !eventsLoading && chatEvents.length === 0 && "flex flex-col justify-end",
        )}
      >
        {eventsLoading ? (
          <div className="space-y-4">
            <Skeleton className="ml-auto h-20 w-3/4" />
            <Skeleton className="h-20 w-3/4" />
            <Skeleton className="h-20 w-2/3" />
          </div>
        ) : chatEvents.length === 0 ? (
          <div className="flex flex-col items-center justify-end text-center text-muted-foreground">
            <div className={chatSurfaceStyles.emptyStateCard}>
              <div className="mx-auto mb-4 flex h-10 w-10 items-center justify-center border border-border/70 bg-background text-muted-foreground">
                <Bot className="h-5 w-5 opacity-65" />
              </div>
              <p className="text-lg font-medium text-foreground">No messages yet</p>
              <p className="mt-1 text-sm">Start with a prompt, screenshot, or slash command.</p>
            </div>
          </div>
        ) : (
          <div className="space-y-6">
            {loadingOlderEvents && (
              <div className="flex items-center justify-center py-3">
                <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
                <span className="ml-2 text-xs text-muted-foreground">
                  Loading older messages...
                </span>
              </div>
            )}
            {chatEvents.map((event) => {
              if (event.type === "tool.started" || event.type === "tool.completed") {
                return null;
              }

              if (event.type === "act.completed") {
                return null;
              }

              if (event.type === "context.compacted") {
                const compactedData = event.data as import("@/lib/api/types").ContextCompactedData;
                return <CompactionDivider key={event.id} data={compactedData} />;
              }

              if (event.type === "act.started") {
                const group = actGroupsByStartEventId.get(event.id);
                if (!group || group.rows.length === 0) return null;
                return (
                  <div key={event.id} className="space-y-3">
                    <div className="ml-9 space-y-1">
                      <ToolActivityTimelineGroup
                        headline={group.headline}
                        completedHeadline={group.completedHeadline}
                        rows={group.rows}
                      />
                    </div>
                    {renderTurnDivider(event.id)}
                  </div>
                );
              }

              if (event.type === "tool.call_requested") {
                const reqData = event.data as ToolCallRequestedData;
                if (!reqData.tool_calls || reqData.tool_calls.length === 0) return null;

                if (reqData.headline || reqData.tool_summaries?.length) {
                  const rows: TimelineToolRow[] = (reqData.tool_summaries ?? []).map((summary) => ({
                    id: summary.id,
                    label: summary.narration ?? summary.display_name ?? summary.name,
                    state: "waiting",
                  }));

                  if (rows.length > 0) {
                    return (
                      <div key={event.id} className="space-y-3">
                        <div className="ml-9 space-y-1">
                          <ToolActivityTimelineGroup
                            headline={reqData.headline ?? "Waiting on tools"}
                            rows={rows}
                          />
                        </div>
                        {renderTurnDivider(event.id)}
                      </div>
                    );
                  }
                }

                // Check for setup_connection tool calls (inline connection prompt)
                const connectionCalls = reqData.tool_calls.filter(
                  (tc) => tc.name === "setup_connection",
                );
                const otherCalls = reqData.tool_calls.filter(
                  (tc) => tc.name !== "setup_connection",
                );

                return (
                  <div key={event.id} className="space-y-3">
                    <div className="ml-9 space-y-1">
                      {connectionCalls.map((tc) => (
                        <SetupConnectionToolCall
                          key={tc.id}
                          sessionId={sessionId}
                          toolCallId={tc.id}
                          provider={(tc.arguments as { provider?: string })?.provider ?? "unknown"}
                          toolResultsMap={toolResultsMap}
                        />
                      ))}
                      {otherCalls.length > 0 && (
                        <ToolActivityGroup
                          toolCalls={otherCalls}
                          toolResultsMap={toolResultsMap}
                          mode="client"
                        />
                      )}
                    </div>
                    {renderTurnDivider(event.id)}
                  </div>
                );
              }

              const isUser = event.type === "input.message";
              const data = event.data as InputMessageData | OutputMessageCompletedData;
              const textContent = getMessageText(data);
              const toolCalls = isUser
                ? []
                : getToolCalls(data as OutputMessageCompletedData).filter(
                    (toolCall) => !clientRequestedToolCallIds.has(toolCall.id),
                  );
              const images = data.message?.content ? getMessageImages(data.message.content) : [];
              const isScheduleTriggered = isUser && data.message?.metadata?.source === "schedule";
              const isToolOnlyMessage =
                !isUser && toolCalls.length > 0 && !textContent && images.length === 0;

              if (isToolOnlyMessage) {
                if (hasNarratedActEvents) {
                  return null;
                }

                return (
                  <div key={event.id} className="space-y-3">
                    <div className="ml-9 space-y-1">
                      <ToolActivityGroup toolCalls={toolCalls} toolResultsMap={toolResultsMap} />
                    </div>
                    {renderTurnDivider(event.id)}
                  </div>
                );
              }

              return (
                <div key={event.id} className="space-y-2">
                  {(textContent || images.length > 0) && (
                    <div className={`flex ${isUser ? "justify-end" : "justify-start"}`}>
                      {isUser ? (
                        <div className={chatSurfaceStyles.userMessage}>
                          {isScheduleTriggered && (
                            <div className="mb-1 flex items-center gap-1 text-[10px] uppercase tracking-[0.22em] text-muted-foreground">
                              <CalendarClock className="h-3 w-3" />
                              <span>Scheduled</span>
                            </div>
                          )}
                          <div className="flex items-start gap-2">
                            <div className="flex-1 space-y-2">
                              {textContent && <p className="whitespace-pre-wrap">{textContent}</p>}
                              {images.length > 0 && (
                                <div className="mt-2 flex flex-wrap gap-2">
                                  {images.map((img) => (
                                    <MessageImage
                                      key={img.image_id}
                                      imageId={img.image_id}
                                      filename={img.filename}
                                    />
                                  ))}
                                </div>
                              )}
                            </div>
                            <MessageInfoIcon event={event} />
                          </div>
                        </div>
                      ) : (
                        <div className={chatSurfaceStyles.agentMessageRow}>
                          <div className={chatSurfaceStyles.agentIcon}>
                            <Bot className="h-3.5 w-3.5" />
                          </div>
                          <div className="flex flex-1 items-start gap-2">
                            <div className={chatSurfaceStyles.agentMessage}>
                              {textContent && <MessageContent text={textContent} />}
                              {images.length > 0 && (
                                <div className="mt-2 flex flex-wrap gap-2">
                                  {images.map((img) => (
                                    <MessageImage
                                      key={img.image_id}
                                      imageId={img.image_id}
                                      filename={img.filename}
                                    />
                                  ))}
                                </div>
                              )}
                            </div>
                            <MessageInfoIcon event={event} />
                          </div>
                        </div>
                      )}
                    </div>
                  )}

                  {toolCalls.length > 0 && !hasNarratedActEvents && (
                    <div className="ml-9 space-y-1">
                      <ToolActivityGroup toolCalls={toolCalls} toolResultsMap={toolResultsMap} />
                    </div>
                  )}

                  {renderTurnDivider(event.id)}
                </div>
              );
            })}
          </div>
        )}

        {(isThinking || streamingText) && (
          <div className="mt-6 flex justify-start">
            <div className={chatSurfaceStyles.agentMessageRow}>
              <div className={chatSurfaceStyles.agentIcon}>
                <Bot className="h-3.5 w-3.5" />
              </div>
              <div className={chatSurfaceStyles.agentMessage}>
                {streamingIteration && streamingIteration > 1 && (
                  <div className="mb-1 text-xs text-muted-foreground">
                    Iteration {streamingIteration}
                  </div>
                )}
                {isThinking && !streamingText ? (
                  <ThinkingIndicator />
                ) : streamingText ? (
                  <StreamingMessage text={streamingText} />
                ) : null}
              </div>
            </div>
          </div>
        )}

        <div ref={messagesEndRef} />

        {hasNewMessages && (
          <button
            type="button"
            onClick={() => {
              scrollToBottom("smooth");
              setHasNewMessages(false);
            }}
            className={chatSurfaceStyles.floatingNotice}
          >
            <ArrowDown className="h-3 w-3" />
            New messages
          </button>
        )}
      </div>

      <div className={chatSurfaceStyles.composerSection}>
        <form onSubmit={handleSubmit} className="space-y-3">
          <input
            ref={fileInputRef}
            type="file"
            accept={ALLOWED_IMAGE_TYPES.join(",")}
            multiple
            className="hidden"
            onChange={handleFileChange}
          />

          {hasImages && <ImageAttachments images={pendingImages} onRemove={removeImage} />}

          <div
            className={cn(
              chatSurfaceStyles.composerInputShell,
              isDraggingOver && "bg-[hsl(var(--accent)/0.07)] ring-1 ring-accent/60",
            )}
            onDragOver={(e) => {
              e.preventDefault();
              e.stopPropagation();
            }}
            onDragEnter={(e) => {
              e.preventDefault();
              e.stopPropagation();
              setIsDraggingOver(true);
            }}
            onDragLeave={(e) => {
              e.preventDefault();
              e.stopPropagation();
              if (!e.currentTarget.contains(e.relatedTarget as Node)) {
                setIsDraggingOver(false);
              }
            }}
            onDrop={(e) => {
              e.preventDefault();
              e.stopPropagation();
              setIsDraggingOver(false);
              const files = e.dataTransfer?.files;
              if (files && files.length > 0) {
                const imageFiles = Array.from(files).filter((f) => f.type.startsWith("image/"));
                if (imageFiles.length > 0) {
                  addFiles(imageFiles);
                }
              }
            }}
          >
            <CommandAutocomplete
              commands={commands}
              inputValue={inputValue}
              visible={showCommands}
              onSelect={handleCommandSelect}
              onDismiss={() => setShowCommands(false)}
            />
            <Textarea
              ref={textareaRef}
              value={inputValue}
              onChange={(e) => {
                const val = e.target.value;
                setInputValue(val);
                setShowCommands(hasCommands && shouldShowCommandAutocomplete(val));
              }}
              onKeyDown={handleKeyDown}
              onPaste={(e) => {
                const items = e.clipboardData?.items;
                if (!items) return;
                const imageFiles: File[] = [];
                for (const item of Array.from(items)) {
                  if (item.type.startsWith("image/")) {
                    const file = item.getAsFile();
                    if (file) imageFiles.push(file);
                  }
                }
                if (imageFiles.length > 0) {
                  e.preventDefault();
                  addFiles(imageFiles);
                }
              }}
              placeholder={inputPlaceholder}
              className={chatSurfaceStyles.composerTextarea}
            />
          </div>

          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex flex-wrap items-center gap-3">
              <Button
                type="button"
                variant="outline"
                size="icon-lg"
                className={chatSurfaceStyles.composerIconButton}
                onClick={() => fileInputRef.current?.click()}
                title="Attach images (PNG, JPEG, GIF, WebP)"
              >
                <ImagePlus className="icon-sharp h-4 w-4" />
              </Button>

              <div className={chatSurfaceStyles.composerControlChip}>
                <span className="shrink-0 text-[10px] uppercase tracking-[0.22em] text-muted-foreground">
                  Model
                </span>
                <Select
                  value={selectedModelId}
                  onValueChange={(value) => {
                    hasUserSelectedModel.current = true;
                    setSelectedModelId(value);
                  }}
                >
                  <SelectTrigger
                    size="sm"
                    className="h-7 w-[156px] min-w-0 border-0 bg-transparent px-0 py-0 text-sm shadow-none focus-visible:ring-0 data-[size=sm]:h-7 [&_svg]:icon-sharp"
                  >
                    <SelectValue>{modelTriggerLabel}</SelectValue>
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="">{defaultModelOptionLabel}</SelectItem>
                    {llmModels.map((model) => (
                      <SelectItem key={model.id} value={model.id}>
                        {model.display_name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              {supportsReasoning && reasoningEffortConfig && (
                <div className={chatSurfaceStyles.composerControlChip}>
                  <Brain className="icon-sharp h-3.5 w-3.5 text-muted-foreground" />
                  <span className="shrink-0 text-[10px] uppercase tracking-[0.22em] text-muted-foreground">
                    Reasoning
                  </span>
                  <Select
                    value={reasoningEffort}
                    onValueChange={(value) => setReasoningEffort(value as typeof reasoningEffort)}
                  >
                    <SelectTrigger
                      size="sm"
                      className="h-7 w-[116px] min-w-0 border-0 bg-transparent px-0 py-0 text-sm shadow-none focus-visible:ring-0 data-[size=sm]:h-7 [&_svg]:icon-sharp"
                    >
                      <SelectValue>
                        {reasoningEffort ? getReasoningEffortName(reasoningEffort) : "Default"}
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

            <div className="flex items-center gap-2">
              {isActive && !inputValue.trim() && !hasImages && (
                <Button
                  type="button"
                  size="icon-lg"
                  variant="destructive"
                  className={chatSurfaceStyles.composerDangerButton}
                  disabled={cancelCurrentTurn.isPending}
                  onClick={() => cancelCurrentTurn.mutate()}
                  title="Cancel current turn"
                >
                  {cancelCurrentTurn.isPending ? (
                    <Loader2 className="h-4 w-4 animate-spin" />
                  ) : (
                    <StopCircle className="icon-sharp h-4 w-4" />
                  )}
                </Button>
              )}

              <Button
                type="submit"
                size="icon-lg"
                className={chatSurfaceStyles.composerSubmitButton}
                disabled={!canSubmit}
                title={isUploading ? "Uploading images..." : undefined}
              >
                {sendMessage.isPending || sendMessageWithImages.isPending ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : isUploading ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Send className="icon-sharp h-4 w-4" />
                )}
              </Button>
            </div>
          </div>
        </form>
      </div>
    </>
  );
}
