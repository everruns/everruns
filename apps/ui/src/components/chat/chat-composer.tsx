/**
 * Decisions:
 * - Keep submit/readiness logic in one component so the parent only coordinates network state.
 * - Preserve slash-command UX: autocomplete owns keyboard navigation whenever visible.
 * - Image upload affordances stay next to text entry because they share send readiness.
 * - When no model resolves, lock the whole composer (text, attachments, voice, drop zone) instead
 *   of only the send button: typing a message that can never be sent is a dead end. The model menu
 *   stays enabled because it is how the user recovers.
 */
"use client";

import { AtSign, ImagePlus, Loader2, Mic, MicOff, Send, StopCircle, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { ModelEffortMenu } from "@/components/chat/model-effort-menu";
import { ImageAttachments } from "@/components/chat/image-attachments";
import {
  CommandAutocomplete,
  shouldShowCommandAutocomplete,
} from "@/components/chat/command-autocomplete";
import {
  getMentionQuery,
  ParticipantMentionAutocomplete,
  type MentionQuery,
  type ParticipantMentionOption,
} from "@/components/chat/participant-mention-autocomplete";
import { chatSurfaceStyles } from "@/components/chat/chat-surface";
import { COMPOSER_AUTOCOMPLETE_LISTBOX_ID } from "@/components/chat/composer-autocomplete";
import { cn } from "@/lib/utils";
import { ALLOWED_IMAGE_TYPES } from "@/lib/api/types";
import type {
  CommandDescriptor,
  Controls,
  Model,
  ReasoningEffortConfig,
  VerbosityConfig,
} from "@/lib/api/types";
import type { PendingImage } from "@/lib/api/images";
import { useEffect, useRef, useState } from "react";
import { useLocale } from "@/providers/locale-provider";

export function ChatComposer({
  commands,
  models,
  inputValue,
  onInputChange,
  onSubmit,
  onCommandSelect,
  mentionOptions = [],
  selectedMention = null,
  onMentionChange,
  pendingImages,
  hasImages,
  removeImage,
  addFiles,
  isDraggingOver,
  dropZoneProps,
  handlePaste,
  placeholder,
  selectedModelId,
  recentModels,
  onModelChange,
  modelTriggerLabel,
  defaultModelOptionLabel,
  supportsReasoning,
  reasoningEffort,
  reasoningEffortConfig,
  defaultEffortName,
  getReasoningEffortName,
  onReasoningEffortChange,
  supportsVerbosity,
  verbosity,
  verbosityConfig,
  defaultVerbosityName,
  getVerbosityName,
  onVerbosityChange,
  isActive,
  cancelCurrentTurn,
  canSubmit,
  modelReady,
  modelLoading,
  isUploading,
  sendPending,
  textareaRef,
  voiceEnabled = false,
  voiceActive = false,
  voicePending = false,
  onToggleVoice,
}: {
  commands: CommandDescriptor[];
  models: Model[];
  inputValue: string;
  onInputChange: (value: string) => void;
  onSubmit: (controls?: Controls) => Promise<void>;
  onCommandSelect: (cmd: CommandDescriptor, controls?: Controls) => Promise<void> | void;
  mentionOptions?: ParticipantMentionOption[];
  selectedMention?: ParticipantMentionOption | null;
  onMentionChange?: (participant: ParticipantMentionOption | null) => void;
  pendingImages: PendingImage[];
  hasImages: boolean;
  removeImage: (tempId: string) => void;
  addFiles: (files: File[]) => void;
  isDraggingOver: boolean;
  dropZoneProps: React.HTMLAttributes<HTMLDivElement>;
  handlePaste: React.ClipboardEventHandler<HTMLTextAreaElement>;
  /** Overrides the generic prompt, e.g. to name the agent being replied to. */
  placeholder?: string;
  selectedModelId: string;
  recentModels: Model[];
  onModelChange: (value: string) => void;
  modelTriggerLabel: string;
  defaultModelOptionLabel: string;
  supportsReasoning: boolean;
  reasoningEffort: string;
  reasoningEffortConfig?: ReasoningEffortConfig;
  defaultEffortName: string;
  getReasoningEffortName: (value: string) => string;
  onReasoningEffortChange: (value: string) => void;
  supportsVerbosity: boolean;
  verbosity: string;
  verbosityConfig?: VerbosityConfig;
  defaultVerbosityName: string;
  getVerbosityName: (value: string) => string;
  onVerbosityChange: (value: string) => void;
  isActive: boolean;
  cancelCurrentTurn: { mutate: () => void; isPending?: boolean };
  canSubmit: boolean;
  modelReady: boolean;
  modelLoading: boolean;
  isUploading: boolean;
  sendPending: boolean;
  textareaRef: React.RefObject<HTMLTextAreaElement | null>;
  voiceEnabled?: boolean;
  voiceActive?: boolean;
  voicePending?: boolean;
  onToggleVoice?: () => void;
}) {
  const { backendLocale, t } = useLocale();
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [showCommands, setShowCommands] = useState(false);
  const [mentionQuery, setMentionQuery] = useState<MentionQuery | null>(null);
  const hasCommands = commands.length > 0;
  const inputPlaceholder =
    placeholder ?? (hasCommands ? t("type_message_or_commands") : t("type_message"));
  // No usable model means nothing can be sent, so the whole entry surface is locked. While the
  // lookup is still in flight the composer stays open so opening a session never flashes disabled.
  const composerDisabled = !modelReady && !modelLoading;

  useEffect(() => {
    setShowCommands(hasCommands && shouldShowCommandAutocomplete(inputValue));
  }, [hasCommands, inputValue]);

  const buildControls = (): Controls | undefined =>
    selectedModelId || reasoningEffort || verbosity
      ? {
          ...(selectedModelId && { model_id: selectedModelId }),
          locale: backendLocale,
          ...(reasoningEffort && supportsReasoning && { reasoning: { effort: reasoningEffort } }),
          ...(verbosity && supportsVerbosity && { verbosity }),
        }
      : { locale: backendLocale };

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!canSubmit) return;
    await onSubmit(buildControls());
    setShowCommands(false);
    setMentionQuery(null);
    textareaRef.current?.focus();
  };

  const handleKeyDown = async (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (showCommands || mentionQuery) return;
    if (
      event.key === "Backspace" &&
      selectedMention &&
      inputValue.length === 0 &&
      onMentionChange
    ) {
      event.preventDefault();
      onMentionChange(null);
      return;
    }
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      if (!canSubmit) return;
      await onSubmit(buildControls());
    }
  };

  const handleFileChange = (event: React.ChangeEvent<HTMLInputElement>) => {
    const files = event.target.files;
    if (files?.length) {
      addFiles(Array.from(files));
    }
    event.target.value = "";
  };

  const selectMention = (option: ParticipantMentionOption) => {
    if (!mentionQuery || !onMentionChange) return;
    const nextValue = `${inputValue.slice(0, mentionQuery.start)}${inputValue.slice(mentionQuery.end)}`;
    onInputChange(nextValue);
    onMentionChange(option);
    setMentionQuery(null);
    window.requestAnimationFrame(() => {
      textareaRef.current?.focus();
      textareaRef.current?.setSelectionRange(mentionQuery.start, mentionQuery.start);
    });
  };

  return (
    <div className={chatSurfaceStyles.composerSection}>
      <form onSubmit={handleSubmit} className="space-y-3">
        {!modelReady && (
          <p className="text-sm text-muted-foreground" role="status">
            {modelLoading ? t("chat_model_loading") : t("chat_model_required")}
          </p>
        )}
        <input
          ref={fileInputRef}
          type="file"
          accept={ALLOWED_IMAGE_TYPES.join(",")}
          multiple
          className="hidden"
          onChange={handleFileChange}
          aria-label="Attach images"
        />

        {hasImages && <ImageAttachments images={pendingImages} onRemove={removeImage} />}

        <div
          className={cn(
            chatSurfaceStyles.composerInputShell,
            !composerDisabled &&
              isDraggingOver &&
              "bg-[hsl(var(--accent)/0.07)] ring-1 ring-accent/60",
            composerDisabled && "opacity-60",
          )}
          {...(composerDisabled ? {} : dropZoneProps)}
        >
          <ParticipantMentionAutocomplete
            options={mentionOptions}
            query={mentionQuery?.query ?? ""}
            visible={mentionQuery !== null}
            onSelect={selectMention}
            onDismiss={() => setMentionQuery(null)}
          />
          <CommandAutocomplete
            commands={commands}
            inputValue={inputValue}
            visible={showCommands}
            onSelect={async (cmd) => {
              setShowCommands(false);
              await onCommandSelect(cmd, buildControls());
              textareaRef.current?.focus();
            }}
            onDismiss={() => setShowCommands(false)}
          />
          {selectedMention && onMentionChange && (
            <div className="flex px-3 pt-3 sm:px-4" data-testid="participant-mention-token">
              <span className="inline-flex max-w-full items-center gap-1 border border-accent/40 bg-[hsl(var(--accent)/0.08)] px-2 py-1 text-xs font-medium text-foreground">
                <AtSign className="h-3 w-3 shrink-0 text-accent" />
                <span className="truncate">{selectedMention.label}</span>
                <button
                  type="button"
                  className="ml-1 inline-flex h-5 w-5 shrink-0 items-center justify-center text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-ring"
                  aria-label={`Remove mention of ${selectedMention.label}`}
                  onClick={() => {
                    onMentionChange(null);
                    textareaRef.current?.focus();
                  }}
                >
                  <X className="h-3 w-3" />
                </button>
              </span>
            </div>
          )}
          <Textarea
            ref={textareaRef}
            value={inputValue}
            onChange={(event) => {
              const value = event.target.value;
              onInputChange(value);
              const nextMentionQuery = getMentionQuery(value, event.target.selectionStart);
              setMentionQuery(nextMentionQuery);
              setShowCommands(
                nextMentionQuery === null && hasCommands && shouldShowCommandAutocomplete(value),
              );
            }}
            onKeyDown={handleKeyDown}
            onPaste={handlePaste}
            role="combobox"
            tabIndex={0}
            disabled={composerDisabled}
            placeholder={inputPlaceholder}
            className={chatSurfaceStyles.composerTextarea}
            aria-autocomplete="list"
            aria-expanded={mentionQuery !== null || showCommands}
            aria-controls={
              mentionQuery || showCommands ? COMPOSER_AUTOCOMPLETE_LISTBOX_ID : undefined
            }
          />
        </div>

        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex flex-wrap items-center gap-3">
            <Button
              type="button"
              variant="outline"
              size="icon-lg"
              className={chatSurfaceStyles.composerIconButton}
              disabled={composerDisabled}
              onClick={() => fileInputRef.current?.click()}
              title={t("attach_images")}
            >
              <ImagePlus className="icon-sharp h-4 w-4" />
            </Button>

            <ModelEffortMenu
              models={models}
              recentModels={recentModels}
              selectedModelId={selectedModelId}
              onModelChange={onModelChange}
              modelTriggerLabel={modelTriggerLabel}
              defaultModelOptionLabel={defaultModelOptionLabel}
              supportsReasoning={supportsReasoning}
              reasoningEffort={reasoningEffort}
              reasoningEffortConfig={reasoningEffortConfig}
              defaultEffortName={defaultEffortName}
              getReasoningEffortName={getReasoningEffortName}
              onReasoningEffortChange={onReasoningEffortChange}
              supportsVerbosity={supportsVerbosity}
              verbosity={verbosity}
              verbosityConfig={verbosityConfig}
              defaultVerbosityName={defaultVerbosityName}
              getVerbosityName={getVerbosityName}
              onVerbosityChange={onVerbosityChange}
            />
          </div>

          <div className="flex items-center gap-2">
            {voiceEnabled && onToggleVoice && (
              <Button
                type="button"
                size="icon-lg"
                variant={voiceActive ? "secondary" : "outline"}
                className={cn(
                  chatSurfaceStyles.composerIconButton,
                  voiceActive && "border-emerald-500/50 text-emerald-600",
                )}
                disabled={voicePending || composerDisabled}
                onClick={onToggleVoice}
                title={voiceActive ? "End voice session" : "Start voice session"}
              >
                {voicePending ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : voiceActive ? (
                  <MicOff className="icon-sharp h-4 w-4" />
                ) : (
                  <Mic className="icon-sharp h-4 w-4" />
                )}
              </Button>
            )}

            {isActive && (
              <Button
                type="button"
                size="icon-lg"
                variant="destructive"
                className={chatSurfaceStyles.composerDangerButton}
                disabled={cancelCurrentTurn.isPending}
                onClick={() => cancelCurrentTurn.mutate()}
                title={t("cancel_current_turn")}
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
              title={isUploading ? t("uploading_images") : undefined}
            >
              {sendPending || isUploading ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Send className="icon-sharp h-4 w-4" />
              )}
            </Button>
          </div>
        </div>
      </form>
    </div>
  );
}
