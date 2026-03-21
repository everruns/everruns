/**
 * Decisions:
 * - Keep submit/readiness logic in one component so the parent only coordinates network state.
 * - Preserve slash-command UX: autocomplete owns keyboard navigation whenever visible.
 * - Image upload affordances stay next to text entry because they share send readiness.
 */
"use client";

import { Brain, ImagePlus, Loader2, Send, StopCircle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ImageAttachments } from "@/components/chat/image-attachments";
import {
  CommandAutocomplete,
  shouldShowCommandAutocomplete,
} from "@/components/chat/command-autocomplete";
import { chatSurfaceStyles } from "@/components/chat/chat-surface";
import { cn } from "@/lib/utils";
import { ALLOWED_IMAGE_TYPES } from "@/lib/api/types";
import type { CommandDescriptor, Controls, LlmModel, ReasoningEffortConfig } from "@/lib/api/types";
import type { PendingImage } from "@/lib/api/images";
import { useEffect, useRef, useState } from "react";

export function ChatComposer({
  commands,
  llmModels,
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
  onModelChange,
  modelTriggerLabel,
  defaultModelOptionLabel,
  supportsReasoning,
  reasoningEffort,
  reasoningEffortConfig,
  defaultEffortName,
  getReasoningEffortName,
  onReasoningEffortChange,
  isActive,
  cancelCurrentTurn,
  canSubmit,
  isUploading,
  sendPending,
}: {
  commands: CommandDescriptor[];
  llmModels: LlmModel[];
  inputValue: string;
  onInputChange: (value: string) => void;
  onSubmit: (controls?: Controls) => Promise<void>;
  onCommandSelect: (cmd: CommandDescriptor, controls?: Controls) => Promise<void> | void;
  pendingImages: PendingImage[];
  hasImages: boolean;
  removeImage: (tempId: string) => void;
  addFiles: (files: File[]) => void;
  isDraggingOver: boolean;
  dropZoneProps: Record<string, unknown>;
  handlePaste: React.ClipboardEventHandler<HTMLTextAreaElement>;
  selectedModelId: string;
  onModelChange: (value: string) => void;
  modelTriggerLabel: string;
  defaultModelOptionLabel: string;
  supportsReasoning: boolean;
  reasoningEffort: string;
  reasoningEffortConfig?: ReasoningEffortConfig;
  defaultEffortName: string;
  getReasoningEffortName: (value: string) => string;
  onReasoningEffortChange: (value: string) => void;
  isActive: boolean;
  cancelCurrentTurn: { mutate: () => void; isPending?: boolean };
  canSubmit: boolean;
  isUploading: boolean;
  sendPending: boolean;
}) {
  const fileInputRef = useRef<HTMLInputElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [showCommands, setShowCommands] = useState(false);
  const hasCommands = commands.length > 0;
  const inputPlaceholder = hasCommands
    ? "Type a message or / for commands... (Enter to send)"
    : "Type a message... (Enter to send)";

  useEffect(() => {
    if (!hasCommands && showCommands) {
      setShowCommands(false);
    }
  }, [hasCommands, showCommands]);

  const buildControls = (): Controls | undefined =>
    selectedModelId || reasoningEffort
      ? {
          ...(selectedModelId && { model_id: selectedModelId }),
          ...(reasoningEffort && supportsReasoning && { reasoning: { effort: reasoningEffort } }),
        }
      : undefined;

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
              title="Attach images (PNG, JPEG, GIF, WebP)"
            >
              <ImagePlus className="icon-sharp h-4 w-4" />
            </Button>

            <div className={chatSurfaceStyles.composerControlChip}>
              <span className="shrink-0 text-[10px] uppercase tracking-[0.22em] text-muted-foreground">
                Model
              </span>
              <Select value={selectedModelId} onValueChange={onModelChange}>
                <SelectTrigger
                  size="sm"
                  nativeButton={false}
                  render={<div />}
                  className="h-7 w-[156px] min-w-0 cursor-pointer border-0 bg-transparent px-0 py-0 text-sm shadow-none focus-visible:ring-0 data-[size=sm]:h-7 [&_svg]:icon-sharp"
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
                  onValueChange={(value) => onReasoningEffortChange(value)}
                >
                  <SelectTrigger
                    size="sm"
                    nativeButton={false}
                    render={<div />}
                    className="h-7 w-[116px] min-w-0 cursor-pointer border-0 bg-transparent px-0 py-0 text-sm shadow-none focus-visible:ring-0 data-[size=sm]:h-7 [&_svg]:icon-sharp"
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
