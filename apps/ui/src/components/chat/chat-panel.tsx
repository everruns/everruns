"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { ArrowDown, Bot, Loader2 } from "lucide-react";
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
import { executeSessionCommand } from "@/lib/api/commands";
import { sendUserMessageWithImages } from "@/lib/api/messages";
import { useMutation } from "@tanstack/react-query";
import { chatSurfaceStyles } from "@/components/chat/chat-surface";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";
import { ChatMessageList } from "@/components/chat/chat-message-list";
import { ChatComposer } from "@/components/chat/chat-composer";
import { MessageContent } from "@/components/chat/message-content";
import { StreamingMessage } from "@/components/streaming-message";
import { ThinkingIndicator } from "@/components/thinking-indicator";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useLocale } from "@/providers/locale-provider";

interface ParsedSystemCommand {
  command: CommandDescriptor;
  argumentsText: string;
}

interface BtwOverlayState {
  question: string;
  answer: string;
  error: string | null;
  pending: boolean;
}

function commandRequiresArguments(command: CommandDescriptor): boolean {
  return (command.args ?? []).some((arg) => arg.required);
}

function parseSystemCommandInvocation(
  input: string,
  commands: CommandDescriptor[],
): ParsedSystemCommand | null {
  const trimmed = input.trim();
  const match = /^\/([^\s]+)(?:\s+(.*))?$/.exec(trimmed);
  if (!match) return null;

  const [, name, rawArguments = ""] = match;
  const command = commands.find((cmd) => cmd.source === "system" && cmd.name === name);
  if (!command) return null;

  const argumentsText = rawArguments.trim();
  return {
    command,
    argumentsText,
  };
}

export function ChatPanel() {
  const { t } = useLocale();
  const {
    agentId,
    events,
    sessionId,
    llmModel,
    chatEvents,
    toolResultsMap,
    toolProgressMap,
    toolOutputMap,
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
  const [submitError, setSubmitError] = useState<string | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

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
  const [btwOverlay, setBtwOverlay] = useState<BtwOverlayState | null>(null);

  useEffect(() => {
    if (!eventsLoading) {
      const focusTimer = window.setTimeout(() => {
        textareaRef.current?.focus();
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
  const executeCommand = useMutation({
    mutationFn: async ({
      name,
      argumentsText,
      controls,
    }: {
      name: string;
      argumentsText?: string;
      controls?: Controls;
    }) =>
      executeSessionCommand(sessionId, {
        name,
        arguments: argumentsText,
        controls,
      }),
  });

  const canSubmit =
    (inputValue.trim().length > 0 || hasImages) &&
    allUploaded &&
    !sendMessage.isPending &&
    !sendMessageWithImages.isPending &&
    !executeCommand.isPending;

  const closeBtwOverlay = useCallback(() => {
    setBtwOverlay(null);
    textareaRef.current?.focus();
  }, []);

  useEffect(() => {
    if (!btwOverlay) return;

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") {
        return;
      }
      closeBtwOverlay();
    };

    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [btwOverlay, closeBtwOverlay]);

  const runSystemCommand = useCallback(
    async (command: CommandDescriptor, argumentsText: string, controls?: Controls) => {
      const trimmedArguments = argumentsText.trim();
      if (commandRequiresArguments(command) && trimmedArguments.length === 0) {
        setInputValue(`/${command.name} `);
        textareaRef.current?.focus();
        return;
      }

      if (command.name === "btw") {
        setBtwOverlay({
          question: trimmedArguments,
          answer: "",
          error: null,
          pending: true,
        });
      }

      setInputValue("");
      persistSelection();

      try {
        const result = await executeCommand.mutateAsync({
          name: command.name,
          argumentsText: trimmedArguments || undefined,
          controls,
        });

        if (command.name === "btw") {
          setBtwOverlay({
            question: trimmedArguments,
            answer: result.message,
            error: result.success ? null : result.message,
            pending: false,
          });
        }
      } catch (error) {
        console.error(`Failed to execute /${command.name}:`, error);
        if (command.name === "btw") {
          setBtwOverlay({
            question: trimmedArguments,
            answer: "",
            error: "Failed to answer side question.",
            pending: false,
          });
        }
      }
    },
    [executeCommand, persistSelection, setInputValue],
  );

  const submitMessage = async (controls?: Controls) => {
    if (!canSubmit) return;
    setSubmitError(null);

    const parsedSystemCommand = hasImages
      ? null
      : parseSystemCommandInvocation(inputValue, commands);
    if (parsedSystemCommand) {
      await runSystemCommand(
        parsedSystemCommand.command,
        parsedSystemCommand.argumentsText,
        controls,
      );
      return;
    }

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
      setSubmitError(error instanceof Error ? error.message : "Failed to send message.");
    }
  };

  const handleCommandSelect = useCallback(
    async (cmd: CommandDescriptor, controls?: Controls) => {
      if (cmd.source === "system") {
        if (commandRequiresArguments(cmd)) {
          setInputValue(`/${cmd.name} `);
          textareaRef.current?.focus();
          return;
        }

        await runSystemCommand(cmd, "", controls);
        return;
      }

      setInputValue(`/${cmd.name} `);
    },
    [runSystemCommand],
  );

  return (
    <>
      <div
        ref={scrollContainerRef}
        onScroll={handleScrollUp}
        className={cn(
          "relative flex-1 overflow-y-auto bg-background bg-brand-dots px-3 py-4 sm:px-4",
          !eventsLoading && chatEvents.length === 0 && "flex flex-col justify-end",
        )}
      >
        <ChatMessageList
          events={events}
          chatEvents={chatEvents}
          sessionId={sessionId}
          toolResultsMap={toolResultsMap}
          toolProgressMap={toolProgressMap}
          toolOutputMap={toolOutputMap}
          eventsLoading={eventsLoading}
          hasMoreEvents={hasMoreEvents}
          loadingOlderEvents={loadingOlderEvents}
          getMessageText={getMessageText}
          getToolCalls={getToolCalls}
        />

        {(isThinking || streamingText) && (
          <div className="mt-4 flex justify-start">
            <div className={chatSurfaceStyles.agentMessageRow}>
              <div className={chatSurfaceStyles.agentIcon}>
                <Bot className="h-3 w-3" />
              </div>
              <div className={chatSurfaceStyles.agentMessage}>
                {streamingIteration && streamingIteration > 1 && (
                  <div className="mb-1 text-xs text-muted-foreground">
                    {t("iteration", { value: streamingIteration })}
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

        {submitError && (
          <div className="mt-4">
            <ChatErrorAlert message={submitError} />
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
            {t("new_messages")}
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
        sendPending={
          sendMessage.isPending || sendMessageWithImages.isPending || executeCommand.isPending
        }
        textareaRef={textareaRef}
      />

      <Dialog open={!!btwOverlay} onOpenChange={(open) => !open && closeBtwOverlay()}>
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>/btw</DialogTitle>
            <DialogDescription>
              Side question about the current session. This answer is ephemeral and does not alter
              the main chat history.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-4">
            <div className="rounded-sm border border-border bg-muted/30 p-3">
              <div className="mb-1 text-[11px] font-medium uppercase tracking-[0.2em] text-muted-foreground">
                Question
              </div>
              <div className="text-sm text-foreground">{btwOverlay?.question}</div>
            </div>

            <div className="rounded-sm border border-border p-4">
              <div className="mb-3 text-[11px] font-medium uppercase tracking-[0.2em] text-muted-foreground">
                Answer
              </div>
              {btwOverlay?.pending ? (
                <div className="flex items-center gap-2 text-sm text-muted-foreground">
                  <Loader2 className="h-4 w-4 animate-spin" />
                  Thinking...
                </div>
              ) : btwOverlay?.error ? (
                <div className="text-sm text-destructive">{btwOverlay.error}</div>
              ) : btwOverlay?.answer ? (
                <MessageContent text={btwOverlay.answer} />
              ) : null}
            </div>
          </div>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={closeBtwOverlay}>
              Dismiss
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
