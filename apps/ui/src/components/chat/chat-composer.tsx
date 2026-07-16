/**
 * Decisions:
 * - Keep submit/readiness logic in one component so the parent only coordinates network state.
 * - Preserve slash-command UX: autocomplete owns keyboard navigation whenever visible.
 * - Image upload affordances stay next to text entry because they share send readiness.
 */
"use client";

import { ImagePlus, Loader2, Mic, MicOff, Send, StopCircle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { ModelEffortMenu } from "@/components/chat/model-effort-menu";
import { ImageAttachments } from "@/components/chat/image-attachments";
import {
  CommandAutocomplete,
  shouldShowCommandAutocomplete,
} from "@/components/chat/command-autocomplete";
import { chatSurfaceStyles } from "@/components/chat/chat-surface";
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
  pendingImages,
  hasImages,
  removeImage,
  addFiles,
  isDraggingOver,
  dropZoneProps,
  handlePaste,
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
  pendingImages: PendingImage[];
  hasImages: boolean;
  removeImage: (tempId: string) => void;
  addFiles: (files: File[]) => void;
  isDraggingOver: boolean;
  dropZoneProps: React.HTMLAttributes<HTMLDivElement>;
  handlePaste: React.ClipboardEventHandler<HTMLTextAreaElement>;
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
  const hasCommands = commands.length > 0;
  const inputPlaceholder = hasCommands ? t("type_message_or_commands") : t("type_message");

  useEffect(() => {
    if (!hasCommands && showCommands) {
      setShowCommands(false);
    }
  }, [hasCommands, showCommands]);

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
    textareaRef.current?.focus();
  };

  const handleKeyDown = async (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (showCommands) return;
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
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

  return (
    <div className={chatSurfaceStyles.composerSection}>
      <form onSubmit={handleSubmit} className="space-y-3">
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
            isDraggingOver && "bg-[hsl(var(--accent)/0.07)] ring-1 ring-accent/60",
          )}
          {...dropZoneProps}
        >
          <CommandAutocomplete
            commands={commands}
            inputValue={inputValue}
            visible={showCommands}
            onSelect={async (cmd) => {
              setShowCommands(false);
              await onCommandSelect(cmd, buildControls());
            }}
            onDismiss={() => setShowCommands(false)}
          />
          <Textarea
            ref={textareaRef}
            value={inputValue}
            onChange={(event) => {
              const value = event.target.value;
              onInputChange(value);
              setShowCommands(hasCommands && shouldShowCommandAutocomplete(value));
            }}
            onKeyDown={handleKeyDown}
            onPaste={handlePaste}
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
                disabled={voicePending}
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
