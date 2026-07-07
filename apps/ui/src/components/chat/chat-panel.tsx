"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ArrowDown, Bot, Loader2 } from "lucide-react";
import type { CommandDescriptor, Controls } from "@/lib/api/types";
import { getEventData } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { useSessionContext } from "@/app/(main)/sessions/[sessionId]/session-context";
import {
  useImageAttachments,
  useImageDropZone,
  useMessageScrollerVisibility,
  useModels,
  useScrollManager,
  useSessionCommands,
  useTurnKeyboardNavigation,
} from "@/hooks";
import { useChatModelSelection } from "@/hooks/use-chat-model-selection";
import { executeSessionCommand } from "@/lib/api/commands";
import { ApiError } from "@/lib/api/client";
import { sendUserMessageWithImages } from "@/lib/api/messages";
import { endSessionVoice, startSessionVoice } from "@/lib/api/voice";
import { useMutation } from "@tanstack/react-query";
import { chatSurfaceStyles } from "@/components/chat/chat-surface";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";
import { ChatMessageList } from "@/components/chat/chat-message-list";
import { ChatNavRail, type ChatNavAnchor } from "@/components/chat/chat-nav-rail";
import { ChatComposer } from "@/components/chat/chat-composer";
import { MessageContent } from "@/components/chat/message-content";
import { SessionTaskChips } from "@/components/session/session-task-chips";
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
import { useFeatureFlag } from "@/providers/feature-flags-provider";

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

interface VoiceErrorState {
  message: string;
  description: string;
}

function commandRequiresArguments(command: CommandDescriptor): boolean {
  return (command.args ?? []).some((arg) => arg.required);
}

function isMicrophonePermissionError(error: unknown): boolean {
  if (!error || typeof error !== "object") return false;

  const name = "name" in error && typeof error.name === "string" ? error.name.toLowerCase() : "";
  if (name === "notallowederror" || name === "securityerror" || name === "permissiondeniederror") {
    return true;
  }

  const message =
    "message" in error && typeof error.message === "string" ? error.message.toLowerCase() : "";
  return (
    typeof DOMException !== "undefined" &&
    error instanceof DOMException &&
    message.includes("permission")
  );
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
  const voiceFeatureEnabled = useFeatureFlag("voice");
  const {
    agentId,
    events,
    sessionId,
    session,
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

  const { data: models = [] } = useModels();
  const [inputValue, setInputValue] = useState("");
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [voiceError, setVoiceError] = useState<VoiceErrorState | null>(null);
  const [voiceState, setVoiceState] = useState<"idle" | "connecting" | "connected">("idle");
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const voiceConnectionIdRef = useRef<string | null>(null);
  const peerConnectionRef = useRef<RTCPeerConnection | null>(null);
  const mediaStreamRef = useRef<MediaStream | null>(null);
  const remoteAudioRef = useRef<HTMLAudioElement | null>(null);

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
    models,
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

  // Turn navigation rail: one marker per user turn. Anchors must line up with
  // the `data-message-anchor` markers that ChatMessageList sets on user rows.
  const navAnchors = useMemo<ChatNavAnchor[]>(() => {
    const anchors: ChatNavAnchor[] = [];
    for (const event of chatEvents) {
      if (event.type !== "input.message") continue;
      const data = getEventData(event, "input.message");
      if (!data) continue;
      const text = getMessageText(data).trim();
      anchors.push({
        id: event.id,
        label: text.length > 80 ? `${text.slice(0, 80)}…` : text,
      });
    }
    return anchors;
  }, [chatEvents, getMessageText]);

  const { currentAnchorId, scrollToAnchor } = useMessageScrollerVisibility(
    scrollContainerRef,
    navAnchors.length,
  );

  // Keyboard turn stepping (Alt+↑/↓, or j/k outside inputs), built on the same
  // anchors and scrollToAnchor the rail uses.
  const navAnchorIds = useMemo(() => navAnchors.map((anchor) => anchor.id), [navAnchors]);
  useTurnKeyboardNavigation({
    anchorIds: navAnchorIds,
    currentAnchorId,
    onNavigate: scrollToAnchor,
  });

  const { isDraggingOver, dropZoneProps, handlePaste } = useImageDropZone({
    onImageFiles: addFiles,
  });

  const { data: commandsData } = useSessionCommands(sessionId);
  const commands = commandsData?.commands ?? [];
  const [btwOverlay, setBtwOverlay] = useState<BtwOverlayState | null>(null);
  const voiceAvailable =
    voiceFeatureEnabled &&
    typeof window !== "undefined" &&
    typeof navigator !== "undefined" &&
    !!navigator.mediaDevices?.getUserMedia &&
    typeof RTCPeerConnection !== "undefined";

  // Task chips: shown only when the leased_resources session feature is present
  // (same gate as the Tasks / resources nav tab).
  const hasTasksFeature = session?.features?.includes("leased_resources") ?? false;
  const sessionBasePath = `/sessions/${sessionId}`;

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

  const cleanupVoiceClient = useCallback(() => {
    peerConnectionRef.current?.close();
    peerConnectionRef.current = null;
    mediaStreamRef.current?.getTracks().forEach((track) => track.stop());
    mediaStreamRef.current = null;
    if (remoteAudioRef.current) {
      remoteAudioRef.current.srcObject = null;
      remoteAudioRef.current.remove();
      remoteAudioRef.current = null;
    }
  }, []);

  const stopVoice = useCallback(
    async (reason = "client_ended") => {
      const voiceConnectionId = voiceConnectionIdRef.current;
      voiceConnectionIdRef.current = null;
      cleanupVoiceClient();
      setVoiceState("idle");
      if (!voiceConnectionId) return;
      try {
        await endSessionVoice(sessionId, voiceConnectionId, reason);
      } catch (error) {
        console.error("Failed to end voice session:", error);
      }
    },
    [cleanupVoiceClient, sessionId],
  );

  useEffect(() => {
    return () => {
      void stopVoice("unmounted");
    };
  }, [stopVoice]);

  const startVoice = useCallback(async () => {
    if (!voiceAvailable || voiceState !== "idle") return;
    setVoiceError(null);
    setVoiceState("connecting");
    try {
      const mediaStream = await navigator.mediaDevices.getUserMedia({ audio: true });
      const peerConnection = new RTCPeerConnection();
      peerConnectionRef.current = peerConnection;
      mediaStreamRef.current = mediaStream;
      mediaStream.getTracks().forEach((track) => peerConnection.addTrack(track, mediaStream));
      const remoteAudio = document.createElement("audio");
      remoteAudio.autoplay = true;
      remoteAudio.setAttribute("playsinline", "true");
      remoteAudioRef.current = remoteAudio;
      peerConnection.ontrack = (event) => {
        remoteAudio.srcObject = event.streams[0];
      };
      const offer = await peerConnection.createOffer();
      await peerConnection.setLocalDescription(offer);
      if (!offer.sdp) {
        throw new Error("Missing local voice offer.");
      }
      const voice = await startSessionVoice(sessionId, {
        sdp: offer.sdp,
        reasoning_effort: reasoningEffort || undefined,
      });
      await peerConnection.setRemoteDescription({ type: "answer", sdp: voice.answer_sdp });
      document.body.appendChild(remoteAudio);
      voiceConnectionIdRef.current = voice.voice_connection_id;
      setVoiceState("connected");
    } catch (error) {
      cleanupVoiceClient();
      setVoiceState("idle");
      if (isMicrophonePermissionError(error)) {
        setVoiceError({
          message: t("voice_microphone_permission_error"),
          description: t("voice_microphone_permission_description"),
        });
      } else if (error instanceof ApiError && error.status >= 500) {
        setVoiceError({
          message: t("voice_service_unavailable_error"),
          description: t("voice_service_unavailable_description"),
        });
      } else {
        setVoiceError({
          message: error instanceof Error ? error.message : "Failed to start voice session.",
          description: t("voice_error_description"),
        });
      }
    }
  }, [cleanupVoiceClient, reasoningEffort, sessionId, t, voiceAvailable, voiceState]);

  const toggleVoice = useCallback(() => {
    if (voiceState === "connected") {
      void stopVoice();
      return;
    }
    if (voiceState === "idle") {
      void startVoice();
    }
  }, [startVoice, stopVoice, voiceState]);

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
      <div className="relative flex min-h-0 flex-1 flex-col">
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

          {voiceError && (
            <div className="mt-4">
              <ChatErrorAlert message={voiceError.message} description={voiceError.description} />
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

        <ChatNavRail
          anchors={navAnchors}
          currentAnchorId={currentAnchorId}
          onJump={scrollToAnchor}
        />
      </div>

      <SessionTaskChips
        sessionId={sessionId}
        basePath={sessionBasePath}
        hasTasksFeature={hasTasksFeature}
      />

      <ChatComposer
        commands={commands}
        models={models}
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
        voiceEnabled={voiceAvailable}
        voiceActive={voiceState === "connected"}
        voicePending={voiceState === "connecting"}
        onToggleVoice={toggleVoice}
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
