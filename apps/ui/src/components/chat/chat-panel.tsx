"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Loader2 } from "lucide-react";
import type { CommandDescriptor, Controls } from "@/lib/api/types";
import type { ConversationStarter } from "@/lib/api/legacy-api-types";
import { useSessionContext } from "@/app/(main)/sessions/[sessionId]/session-context";
import {
  useAgents,
  useFileAttachments,
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
import { useIntelligenceStatus } from "@/hooks/use-intelligence";
import { executeSessionCommand } from "@/lib/api/commands";
import { ApiError } from "@/lib/api/client";
import { startSessionVoice } from "@/lib/api/voice";
import { useVoiceCall } from "@/hooks/use-voice-call";
import { useMutation } from "@tanstack/react-query";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";
import { ChatComposer } from "@/components/chat/chat-composer";
import { NoIntelligenceMessage } from "@/components/chat/no-intelligence-notice";
import { PlatformChatIntroBox } from "@/components/chat/platform-chat-intro";
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
  /** Resolved work retains its transcript; reopen explicitly before continuing. */
  resolvedThread?: boolean;
  onDraftSubmit?: (
    text: string,
    images: Array<{ imageId: string; filename?: string }>,
    files: Array<{ fileId: string; filename?: string }>,
    controls?: Controls,
  ) => Promise<void>;
  /** Hosts with their own context rail may hide the shared participant rail. */
  showParticipants?: boolean;
  /**
   * Who the composer is replying to. The Chats thread surface names the bound
   * agent here; other surfaces leave it unset and keep the generic prompt.
   */
  replyToLabel?: string;
  /** A thread a coordinator started: the composer says "this thread", not the agent. */
  replyInThread?: boolean;
  /**
   * Render inline run cards for work the turns started. Off by default: it costs
   * a task subscription, and only the Chats thread surface wants it.
   */
  showRunCards?: boolean;
  /** Playground keeps the full log visible while retaining the chat composer. */
  collapseWorkLog?: boolean;
  /** Icon name for the Platform Chat intro card (harness icon set). */
  platformIcon?: string | null;
  /** Markdown intro; the intro box renders while the transcript is empty. */
  platformIntro?: string | null;
  /** Conversation starters rendered above the composer while empty. */
  platformStarters?: ConversationStarter[];
}

export function ChatPanel({
  resolvedThread = false,
  replyToLabel,
  replyInThread = false,
  onDraftSubmit,
  showRunCards = false,
  collapseWorkLog = true,
  showParticipants = true,
  platformIcon,
  platformIntro,
  platformStarters = [],
}: ChatPanelProps = {}) {
  const { t } = useLocale();
  const voiceFeatureEnabled = useFeatureFlag("voice");
  const {
    agentId,
    sessionId,
    session,
    chatEvents,
    llmModel,
    llmModelLoading,
    eventsLoading,
    isActive,
    reasoningEffort,
    setReasoningEffort,
    verbosity,
    setVerbosity,
    setIsWaitingForResponse,
    chatSends,
    cancelCurrentTurn,
  } = useSessionContext();

  const { data: models = [], isLoading: modelsLoading } = useModels();
  // One guiding block on a fresh platform thread: the intro + starters render
  // *as* the transcript empty state (centered welcome), so there is no
  // "No messages yet" card, no repeated thread name, and no model-notice
  // paragraph above the composer. The composer's model picker is the single
  // model cue.
  const intelligence = useIntelligenceStatus();
  const showNoIntelligence = !intelligence.isLoading && !intelligence.available;
  const transcriptEmpty =
    !eventsLoading && chatEvents.length === 0 && chatSends.pending.length === 0;
  const showPlatformIntro =
    transcriptEmpty && Boolean(platformIntro || platformStarters.length > 0);
  const { data: participants, refetch: refetchParticipants } = useSessionParticipants(sessionId);
  const { data: agents } = useAgents();
  const [draftSending, setDraftSending] = useState(false);
  const [inputValue, setInputValue] = useState("");
  const [addressedParticipantId, setAddressedParticipantId] = useState<string | null>(null);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [voiceError, setVoiceError] = useState<VoiceErrorState | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

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
    addFiles: addImageFiles,
    removeImage,
    clearImages,
    hasImages,
    isUploading,
  } = useImageAttachments({ sessionId: sessionId || undefined });

  const {
    pendingFiles,
    allUploaded: allFilesUploaded,
    uploadedFileIds,
    addFiles: addFileFiles,
    removeFile: removeFileAttachment,
    clearFiles,
    handlePaste: handleFilePaste,
    hasFiles,
  } = useFileAttachments({ sessionId: sessionId || undefined });

  const supportsPdf =
    (
      selectedModel as unknown as {
        profile?: { modalities?: { input?: string[] } };
      } | null
    )?.profile?.modalities?.input?.includes("pdf") ?? false;

  const addFiles = useCallback(
    (files: File[]) => {
      const isPdf = (f: File) =>
        f.type === "application/pdf" || f.name.toLowerCase().endsWith(".pdf");
      const pdfs = files.filter(isPdf);
      const imgs = files.filter((f) => !isPdf(f));
      if (imgs.length > 0) {
        addImageFiles(imgs);
      }
      if (pdfs.length > 0 && supportsPdf) {
        addFileFiles(pdfs);
      }
    },
    [addImageFiles, addFileFiles, supportsPdf],
  );

  const {
    isDraggingOver,
    dropZoneProps,
    handlePaste: imageHandlePaste,
  } = useImageDropZone({
    onImageFiles: addFiles,
  });

  const handlePaste = useCallback(
    (event: React.ClipboardEvent) => {
      imageHandlePaste(event);
      if (supportsPdf) {
        handleFilePaste(event);
      }
    },
    [imageHandlePaste, handleFilePaste, supportsPdf],
  );

  const modelReady = Boolean(selectedModel || (!selectedModelId && llmModel));
  const modelLoading = selectedModelId ? modelsLoading : llmModelLoading;

  const { data: commandsData } = useSessionCommands(sessionId);
  const commands = commandsData?.commands ?? [];
  const [commandOverlay, setCommandOverlay] = useState<CommandOverlayState | null>(null);
  const activeSessionIdRef = useRef(sessionId);
  const voiceAvailable =
    !!sessionId &&
    voiceFeatureEnabled &&
    session?.source !== "playground" &&
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
    (inputValue.trim().length > 0 || hasImages || hasFiles) &&
    allUploaded &&
    allFilesUploaded &&
    !draftSending &&
    !executeCommand.isPending;

  const placeSessionCall = useCallback(
    async (sdp: string) => ({ sessionId, voice: await startSessionVoice(sessionId, { sdp }) }),
    [sessionId],
  );

  const handleVoiceError = useCallback(
    (error: unknown) => {
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
    },
    [t],
  );

  const {
    state: voiceState,
    start: startVoiceCall,
    stop: stopVoice,
  } = useVoiceCall({ placeCall: placeSessionCall, onError: handleVoiceError });

  // A call belongs to one session: switching sessions hangs it up.
  useEffect(() => {
    return () => {
      void stopVoice("unmounted");
    };
  }, [sessionId, stopVoice]);

  const startVoice = useCallback(async () => {
    if (!voiceAvailable || voiceState !== "idle") return;
    setVoiceError(null);
    await startVoiceCall();
  }, [startVoiceCall, voiceAvailable, voiceState]);

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
      if (onDraftSubmit) {
        setDraftSending(true);
        await onDraftSubmit(inputValue.trim(), uploadedImageIds, uploadedFileIds, controls);
        clearImages();
        clearFiles();
      } else {
        // The turn status row takes over from Enter: the composer clears now,
        // and a send that fails shows "Not delivered" with Retry on its row.
        const send = chatSends.submit({
          text: inputValue.trim(),
          images: hasImages ? uploadedImageIds : [],
          files: hasFiles ? uploadedFileIds : [],
          controls,
          addressedParticipantId,
        });
        clearImages();
        clearFiles();
        persistSelection();
        setInputValue("");
        setAddressedParticipantId(null);
        setIsWaitingForResponse(true);
        send
          .then(() => {
            if (sessionId) void refetchParticipants();
          })
          .catch((error) => console.error("Failed to send message:", error));
        return;
      }

      if (sessionId) void refetchParticipants();
      persistSelection();
      setInputValue("");
      setAddressedParticipantId(null);
      setIsWaitingForResponse(true);
    } catch (error) {
      console.error("Failed to send message:", error);
      setSubmitError(error instanceof Error ? error.message : "Failed to send message.");
    } finally {
      setDraftSending(false);
    }
  };

  // Stop first takes back a send whose turn has not started: before the
  // server acks it the text returns to the composer, after the ack the stop
  // waits for the turn to start. Otherwise it cancels the running turn.
  const stopTurn = useMemo(
    () => ({
      isPending: cancelCurrentTurn.isPending,
      mutate: () => {
        const stopped = chatSends.stop();
        if (typeof stopped === "object") {
          setInputValue((current) => current || stopped.text);
          textareaRef.current?.focus();
          return;
        }
        if (!stopped) cancelCurrentTurn.mutate();
      },
    }),
    [cancelCurrentTurn, chatSends],
  );

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
            collapseWorkLog={collapseWorkLog}
            // Playground sessions cannot be forked, so they get no Branch or rating.
            messageActions={!!session && session.source !== "playground"}
            emptyState={
              showNoIntelligence ? (
                <NoIntelligenceMessage canManage={intelligence.canManage} />
              ) : showPlatformIntro ? (
                <PlatformChatIntroBox
                  icon={platformIcon ?? null}
                  intro={platformIntro ?? null}
                  starters={platformStarters}
                  onSelect={(text) => {
                    setInputValue(text);
                    textareaRef.current?.focus();
                  }}
                />
              ) : undefined
            }
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

          {showNoIntelligence && !transcriptEmpty && (
            <NoIntelligenceMessage canManage={intelligence.canManage} className="mb-3" />
          )}

          {resolvedThread ? (
            <p className="border-t border-border p-4 text-sm text-muted-foreground">
              This thread is resolved. Reopen it to continue.
            </p>
          ) : (
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
              pendingFiles={pendingFiles}
              hasFiles={hasFiles}
              removeFileAttachment={removeFileAttachment}
              supportsPdf={supportsPdf}
              isDraggingOver={isDraggingOver}
              dropZoneProps={dropZoneProps}
              handlePaste={handlePaste}
              placeholder={
                showPlatformIntro
                  ? !modelReady
                    ? t("type_message_pick_model")
                    : undefined
                  : replyInThread
                    ? t("reply_in_thread")
                    : replyToLabel
                      ? t("reply_to", { name: replyToLabel })
                      : undefined
              }
              selectedModelId={selectedModelId}
              usingChatGptPlan={
                selectedModel?.provider_type === "chatgpt" ||
                (!selectedModelId && llmModel?.provider_type === "chatgpt")
              }
              recentModels={recentModels}
              onModelChange={handleModelChange}
              modelTriggerLabel={
                showPlatformIntro && !modelReady ? t("choose_model") : modelTriggerLabel
              }
              defaultModelOptionLabel={defaultModelOptionLabel}
              supportsReasoning={supportsReasoning}
              reasoningEffort={reasoningEffort}
              reasoningEffortConfig={reasoningEffortConfig}
              defaultEffortName={defaultEffortName}
              getReasoningEffortName={getReasoningEffortName}
              onReasoningEffortChange={(value) =>
                setReasoningEffort(value as typeof reasoningEffort)
              }
              supportsVerbosity={supportsVerbosity}
              verbosity={verbosity}
              verbosityConfig={verbosityConfig}
              defaultVerbosityName={defaultVerbosityName}
              getVerbosityName={getVerbosityName}
              onVerbosityChange={(value) => setVerbosity(value as typeof verbosity)}
              isActive={isActive || chatSends.busy}
              cancelCurrentTurn={stopTurn}
              canSubmit={canSubmit}
              modelReady={modelReady}
              modelLoading={modelLoading}
              hideModelNotice={showPlatformIntro}
              isUploading={isUploading}
              sendPending={draftSending || executeCommand.isPending}
              textareaRef={textareaRef}
              voiceEnabled={voiceAvailable}
              voiceActive={voiceState === "connected"}
              voicePending={voiceState === "connecting"}
              onToggleVoice={toggleVoice}
            />
          )}
        </div>

        {showParticipants && <SessionParticipantsRail sessionId={sessionId} />}
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
