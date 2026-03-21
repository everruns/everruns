"use client";

import { useCallback, useEffect, useState } from "react";
import { ArrowDown, Bot } from "lucide-react";
import type { CommandDescriptor, Controls } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { useSessionContext } from "@/app/(main)/sessions/[sessionId]/session-context";
import {
  useImageAttachments,
  useImageDropZone,
  useLlmModels,
  useScrollManager,
  useSessionCommands,
} from "@/hooks";
import { useChatModelSelection } from "@/hooks/use-chat-model-selection";
import { sendUserMessageWithImages } from "@/lib/api/messages";
import { useMutation } from "@tanstack/react-query";
import { chatSurfaceStyles } from "@/components/chat/chat-surface";
import { ChatMessageList } from "@/components/chat/chat-message-list";
import { ChatComposer } from "@/components/chat/chat-composer";
import { StreamingMessage } from "@/components/streaming-message";
import { ThinkingIndicator } from "@/components/thinking-indicator";

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

  const {
    selectedModelId,
    supportsReasoning,
    reasoningEffortConfig,
    defaultEffortName,
    modelTriggerLabel,
    defaultModelOptionLabel,
    getReasoningEffortName,
    handleModelChange,
    persistSelection,
  } = useChatModelSelection({
    agentId,
    sessionId,
    llmModels,
    defaultModel: llmModel,
    reasoningEffort,
    setReasoningEffort,
  });

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

  const { scrollContainerRef, messagesEndRef, hasNewMessages, dismissNewMessages, handleScrollUp } =
    useScrollManager({
      eventCount: chatEvents.length,
      eventsLoaded: !eventsLoading,
      hasMoreEvents,
      loadingOlderEvents,
      loadOlderEvents,
      sessionId,
      scrollDeps: [streamingText, isThinking],
    });

  const { isDraggingOver, dropZoneProps, handlePaste } = useImageDropZone({
    onImageFiles: addFiles,
  });

  const { data: commandsData } = useSessionCommands(sessionId);
  const commands = commandsData?.commands ?? [];

  useEffect(() => {
    if (!eventsLoading) {
      const focusTimer = window.setTimeout(() => {
        const textarea = document.querySelector("textarea");
        if (textarea instanceof HTMLTextAreaElement) {
          textarea.focus();
        }
      }, 0);
      return () => window.clearTimeout(focusTimer);
    }
  }, [eventsLoading]);

  const sendMessageWithImages = useMutation({
    mutationFn: async ({
      text,
      images,
      controls,
    }: {
      text: string;
      images: Array<{ imageId: string; filename?: string }>;
      controls?: Controls;
    }) => sendUserMessageWithImages(sessionId, text, images, controls),
  });

  const canSubmit =
    (inputValue.trim().length > 0 || hasImages) &&
    allUploaded &&
    !sendMessage.isPending &&
    !sendMessageWithImages.isPending;

  const submitMessage = async (controls?: Controls) => {
    if (!canSubmit) return;

    try {
      if (hasImages) {
        await sendMessageWithImages.mutateAsync({
          text: inputValue.trim(),
          images: uploadedImageIds,
          controls,
        });
        clearImages();
      } else {
        await sendMessage.mutateAsync({
          sessionId,
          content: inputValue.trim(),
          controls,
        });
      }

      persistSelection();
      setInputValue("");
      setIsWaitingForResponse(true);
    } catch (error) {
      console.error("Failed to send message:", error);
    }
  };

  const handleCommandSelect = useCallback(
    async (cmd: CommandDescriptor, controls?: Controls) => {
      if (cmd.source === "system") {
        setInputValue(`/${cmd.name}`);
        await sendMessage.mutateAsync({
          sessionId,
          content: `/${cmd.name}`,
          controls,
        });
        setInputValue("");
        setIsWaitingForResponse(true);
        return;
      }

      setInputValue(`/${cmd.name} `);
    },
    [sendMessage, sessionId, setIsWaitingForResponse],
  );

  return (
    <>
      <div
        ref={scrollContainerRef}
        onScroll={handleScrollUp}
        className={cn(
          "relative flex-1 overflow-y-auto bg-background bg-brand-dots px-4 py-5 sm:px-6",
          !eventsLoading && chatEvents.length === 0 && "flex flex-col justify-end",
        )}
      >
        <ChatMessageList
          events={events}
          chatEvents={chatEvents}
          sessionId={sessionId}
          toolResultsMap={toolResultsMap}
          eventsLoading={eventsLoading}
          hasMoreEvents={hasMoreEvents}
          loadingOlderEvents={loadingOlderEvents}
          getMessageText={getMessageText}
          getToolCalls={getToolCalls}
        />

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
            onClick={dismissNewMessages}
            className={chatSurfaceStyles.floatingNotice}
          >
            <ArrowDown className="h-3 w-3" />
            New messages
          </button>
        )}
      </div>

      <ChatComposer
        commands={commands}
        llmModels={llmModels}
        inputValue={inputValue}
        onInputChange={setInputValue}
        onSubmit={submitMessage}
        onCommandSelect={handleCommandSelect}
        pendingImages={pendingImages}
        hasImages={hasImages}
        removeImage={removeImage}
        addFiles={addFiles}
        isDraggingOver={isDraggingOver}
        dropZoneProps={dropZoneProps}
        handlePaste={handlePaste}
        selectedModelId={selectedModelId}
        onModelChange={handleModelChange}
        modelTriggerLabel={modelTriggerLabel}
        defaultModelOptionLabel={defaultModelOptionLabel}
        supportsReasoning={supportsReasoning}
        reasoningEffort={reasoningEffort}
        reasoningEffortConfig={reasoningEffortConfig}
        defaultEffortName={defaultEffortName}
        getReasoningEffortName={getReasoningEffortName}
        onReasoningEffortChange={(value) => setReasoningEffort(value as typeof reasoningEffort)}
        isActive={isActive}
        cancelCurrentTurn={cancelCurrentTurn}
        canSubmit={canSubmit}
        isUploading={isUploading}
        sendPending={sendMessage.isPending || sendMessageWithImages.isPending}
      />
    </>
  );
}
