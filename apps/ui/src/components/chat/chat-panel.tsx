"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Loader2 } from "lucide-react";
import type { CommandDescriptor, Controls } from "@/lib/api/types";
import { useSessionContext } from "@/app/(main)/sessions/[sessionId]/session-context";
import {
  useAgents,
  useImageAttachments,
  useImageDropZone,
  useModels,
  useSessionCommands,
  useSessionParticipants,
} from "@/hooks";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { getSessionParticipantLabel } from "@/lib/session-participant-label";
import type { ParticipantMentionOption } from "@/components/chat/participant-mention-autocomplete";
import { useChatModelSelection } from "@/hooks/use-chat-model-selection";
import { executeSessionCommand } from "@/lib/api/commands";
import { ApiError } from "@/lib/api/client";
import { sendUserMessageWithImages } from "@/lib/api/messages";
import { endSessionVoice, startSessionVoice } from "@/lib/api/voice";
import { useMutation } from "@tanstack/react-query";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";
import { ChatComposer } from "@/components/chat/chat-composer";
import { MessageContent } from "@/components/chat/message-content";
import { SessionTaskChips } from "@/components/session/session-task-chips";
import { SessionParticipantsRail } from "@/components/session/session-participants-rail";
import { SessionTranscript } from "@/components/session/session-transcript";
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

interface CommandOverlayState {
  commandName: string;
  argumentsText: string;
  message: string;
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

export interface ChatPanelProps {
  /**
   * Who the composer is replying to. The Chats thread surface names the bound
   * agent here; other surfaces leave it unset and keep the generic prompt.
   */
  replyToLabel?: string;
  /**
   * Render inline run cards for work the turns started. Off by default: it costs
   * a task subscription, and only the Chats thread surface wants it.
   */
  showRunCards?: boolean;
}

export function ChatPanel({ replyToLabel, showRunCards = false }: ChatPanelProps = {}) {
  const { t } = useLocale();
  const voiceFeatureEnabled = useFeatureFlag("voice");
  const {
    agentId,
    sessionId,
    session,
    llmModel,
    llmModelLoading,
    eventsLoading,
    isActive,
    reasoningEffort,
    setReasoningEffort,
    verbosity,
    setVerbosity,
    setIsWaitingForResponse,
    sendMessage,
    cancelCurrentTurn,
  } = useSessionContext();

  const { data: models = [], isLoading: modelsLoading } = useModels();
  const { data: participants, refetch: refetchParticipants } = useSessionParticipants(sessionId);
  const { data: agents } = useAgents();
  const [inputValue, setInputValue] = useState("");
  const [addressedParticipantId, setAddressedParticipantId] = useState<string | null>(null);
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
    selectedModel,
    recentModels,
    supportsReasoning,
    reasoningEffortConfig,
    defaultEffortName,
    supportsVerbosity,
    verbosityConfig,
    defaultVerbosityName,
    getVerbosityName,
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
    defaultModelLoading: llmModelLoading,
    modelsLoading,
    reasoningEffort,
    setReasoningEffort,
    verbosity,
    setVerbosity,
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

  const modelReady = Boolean(selectedModel || (!selectedModelId && llmModel));
  const modelLoading = selectedModelId ? modelsLoading : llmModelLoading;

  const { isDraggingOver, dropZoneProps, handlePaste } = useImageDropZone({
    onImageFiles: addFiles,
  });

  const { data: commandsData } = useSessionCommands(sessionId);
  const commands = commandsData?.commands ?? [];
  const [commandOverlay, setCommandOverlay] = useState<CommandOverlayState | null>(null);
  const activeSessionIdRef = useRef(sessionId);
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
      addressedParticipantId: addressed,
    }: {
      text: string;
      images: Array<{ imageId: string; filename?: string }>;
      controls?: Controls;
      addressedParticipantId?: string | null;
    }) => sendUserMessageWithImages(sessionId, text, images, controls, addressed),
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
    modelReady &&
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

  useEffect(() => {
    if (activeSessionIdRef.current === sessionId) return;
    activeSessionIdRef.current = sessionId;
    setInputValue("");
    setSubmitError(null);
    setCommandOverlay(null);
    setAddressedParticipantId(null);
    clearImages();
  }, [clearImages, sessionId]);

  const closeCommandOverlay = useCallback(() => {
    setCommandOverlay(null);
    textareaRef.current?.focus();
  }, []);

  useEffect(() => {
    if (!commandOverlay) return;

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") {
        return;
      }
      closeCommandOverlay();
    };

    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [commandOverlay, closeCommandOverlay]);

  const runSystemCommand = useCallback(
    async (command: CommandDescriptor, argumentsText: string, controls?: Controls) => {
      const trimmedArguments = argumentsText.trim();
      if (commandRequiresArguments(command) && trimmedArguments.length === 0) {
        setInputValue(`/${command.name} `);
        textareaRef.current?.focus();
        return;
      }

      const requestSessionId = sessionId;
      setCommandOverlay({
        commandName: command.name,
        argumentsText: trimmedArguments,
        message: "",
        error: null,
        pending: true,
      });

      setInputValue("");
      persistSelection();

      try {
        const result = await executeCommand.mutateAsync({
          name: command.name,
          argumentsText: trimmedArguments || undefined,
          controls,
        });

        if (activeSessionIdRef.current !== requestSessionId) return;
        setCommandOverlay({
          commandName: command.name,
          argumentsText: trimmedArguments,
          message: result.message,
          error: result.success ? null : result.message,
          pending: false,
        });
      } catch (error) {
        console.error(`Failed to execute /${command.name}:`, error);
        if (activeSessionIdRef.current !== requestSessionId) return;
        setCommandOverlay({
          commandName: command.name,
          argumentsText: trimmedArguments,
          message: "",
          error: "Command execution failed. Try again.",
          pending: false,
        });
      }
    },
    [executeCommand, persistSelection, sessionId, setInputValue],
  );

  const agentNameById = useMemo(() => {
    const map = new Map<string, string>();
    for (const agent of agents ?? []) {
      map.set(agent.id, getDisplayName(agent));
    }
    return map;
  }, [agents]);

  const activeAgentParticipants = useMemo(
    () => (participants ?? []).filter((p) => !p.left_at && p.kind === "agent"),
    [participants],
  );
  const addressableParticipants = useMemo(
    () => activeAgentParticipants.filter((p) => p.role !== "host"),
    [activeAgentParticipants],
  );
  const mentionOptions = useMemo<ParticipantMentionOption[]>(() => {
    const labels = addressableParticipants.map((participant) =>
      getSessionParticipantLabel(participant, agentNameById),
    );
    const labelCounts = new Map<string, number>();
    for (const label of labels) labelCounts.set(label, (labelCounts.get(label) ?? 0) + 1);

    return addressableParticipants.map((participant, index) => {
      const label = labels[index];
      const suffix = participant.id.replace(/^part_/, "").slice(-6);
      return {
        id: participant.id,
        label: labelCounts.get(label)! > 1 ? `${label} (${suffix})` : label,
        description: `Guest agent · ${suffix}`,
      };
    });
  }, [addressableParticipants, agentNameById]);

  useEffect(() => {
    if (
      addressedParticipantId &&
      !addressableParticipants.some((p) => p.id === addressedParticipantId)
    ) {
      setAddressedParticipantId(null);
    }
  }, [addressableParticipants, addressedParticipantId]);

  const selectedMention =
    mentionOptions.find((participant) => participant.id === addressedParticipantId) ?? null;

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
          addressedParticipantId,
        });
        clearImages();
      } else {
        await sendMessage.mutateAsync({
          sessionId,
          content: inputValue.trim(),
          controls,
          addressedParticipantId,
        });
      }

      void refetchParticipants();
      persistSelection();
      setInputValue("");
      setAddressedParticipantId(null);
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

        if (modelReady) {
          await runSystemCommand(cmd, "", controls);
        }
        return;
      }

      setInputValue(`/${cmd.name} `);
    },
    [modelReady, runSystemCommand],
  );

  return (
    <>
      <div className="flex min-h-0 flex-1">
        <div className="flex min-h-0 flex-1 flex-col">
          <SessionTranscript
            showRunCards={showRunCards}
            footer={
              <>
                {submitError && (
                  <div className="mt-4">
                    <ChatErrorAlert message={submitError} />
                  </div>
                )}

                {voiceError && (
                  <div className="mt-4">
                    <ChatErrorAlert
                      message={voiceError.message}
                      description={voiceError.description}
                    />
                  </div>
                )}
              </>
            }
          />

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
            mentionOptions={mentionOptions}
            selectedMention={selectedMention}
            onMentionChange={(participant) => setAddressedParticipantId(participant?.id ?? null)}
            pendingImages={pendingImages}
            hasImages={hasImages}
            removeImage={removeImage}
            addFiles={addFiles}
            isDraggingOver={isDraggingOver}
            dropZoneProps={dropZoneProps}
            handlePaste={handlePaste}
            placeholder={replyToLabel ? t("reply_to", { name: replyToLabel }) : undefined}
            selectedModelId={selectedModelId}
            recentModels={recentModels}
            onModelChange={handleModelChange}
            modelTriggerLabel={modelTriggerLabel}
            defaultModelOptionLabel={defaultModelOptionLabel}
            supportsReasoning={supportsReasoning}
            reasoningEffort={reasoningEffort}
            reasoningEffortConfig={reasoningEffortConfig}
            defaultEffortName={defaultEffortName}
            getReasoningEffortName={getReasoningEffortName}
            onReasoningEffortChange={(value) => setReasoningEffort(value as typeof reasoningEffort)}
            supportsVerbosity={supportsVerbosity}
            verbosity={verbosity}
            verbosityConfig={verbosityConfig}
            defaultVerbosityName={defaultVerbosityName}
            getVerbosityName={getVerbosityName}
            onVerbosityChange={(value) => setVerbosity(value as typeof verbosity)}
            isActive={isActive}
            cancelCurrentTurn={cancelCurrentTurn}
            canSubmit={canSubmit}
            modelReady={modelReady}
            modelLoading={modelLoading}
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
        </div>

        <SessionParticipantsRail sessionId={sessionId} />
      </div>

      <Dialog open={!!commandOverlay} onOpenChange={(open) => !open && closeCommandOverlay()}>
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>/{commandOverlay?.commandName}</DialogTitle>
            <DialogDescription>
              This command result is ephemeral and does not alter the main chat history.
            </DialogDescription>
          </DialogHeader>

          <div className="space-y-4">
            {commandOverlay?.argumentsText && (
              <div className="rounded-sm border border-border bg-muted/30 p-3">
                <div className="mb-1 text-[11px] font-medium uppercase tracking-[0.2em] text-muted-foreground">
                  Arguments
                </div>
                <div className="text-sm text-foreground">{commandOverlay.argumentsText}</div>
              </div>
            )}

            <div className="rounded-sm border border-border p-4">
              <div className="mb-3 text-[11px] font-medium uppercase tracking-[0.2em] text-muted-foreground">
                Result
              </div>
              {commandOverlay?.pending ? (
                <div className="flex items-center gap-2 text-sm text-muted-foreground">
                  <Loader2 className="h-4 w-4 animate-spin" />
                  Thinking...
                </div>
              ) : commandOverlay?.error ? (
                <div className="text-sm text-destructive">{commandOverlay.error}</div>
              ) : commandOverlay?.message ? (
                <MessageContent text={commandOverlay.message} />
              ) : null}
            </div>
          </div>

          <DialogFooter>
            <Button type="button" variant="outline" onClick={closeCommandOverlay}>
              Dismiss
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
