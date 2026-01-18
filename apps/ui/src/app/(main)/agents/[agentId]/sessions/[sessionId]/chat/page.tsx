"use client";

import { useState, useRef, useEffect, useMemo } from "react";
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
import { Send, Bot, Loader2, Brain, ImagePlus } from "lucide-react";
import type { Controls, MessageUserData, MessageAgentData, ContentPart } from "@/lib/api/types";
import { isImageFilePart } from "@/lib/api/types";
import { ToolCallCardFromEvent } from "@/components/chat/tool-call-card-from-event";
import { MessageInfoIcon } from "@/components/chat/message-info-icon";
import { ImageAttachments, MessageImage } from "@/components/chat/image-attachments";
import { useSessionContext } from "../session-context";
import { useLlmModels, useImageAttachments } from "@/hooks";
import { sendUserMessageWithImages } from "@/lib/api/messages";
import { useMutation } from "@tanstack/react-query";
import { ALLOWED_IMAGE_TYPES } from "@/lib/api/types";
import { useOrg } from "@/providers/org-provider";

export default function ChatPage() {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;

  const {
    agentId,
    sessionId,
    llmModel,
    chatEvents,
    toolResultsMap,
    eventsLoading,
    reasoningEffort,
    setReasoningEffort,
    setIsWaitingForResponse,
    sendMessage,
    getMessageText,
    getToolCalls,
  } = useSessionContext();

  const { data: llmModels = [] } = useLlmModels();

  const [inputValue, setInputValue] = useState("");
  const [selectedModelId, setSelectedModelId] = useState("");
  const [isDraggingOver, setIsDraggingOver] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const hasUserSelectedModel = useRef(false);
  const modelSelectionStorageKey = useMemo(
    () => `everruns:chat:model-selection:${agentId}:${sessionId}`,
    [agentId, sessionId]
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

  useEffect(() => {
    if (typeof window === "undefined") return;
    const storedSelection = window.localStorage.getItem(modelSelectionStorageKey);
    if (storedSelection !== null && !hasUserSelectedModel.current) {
      setSelectedModelId(storedSelection);
    }
  }, [modelSelectionStorageKey]);

  const selectedModel = useMemo(
    () => llmModels.find((model) => model.id === selectedModelId),
    [llmModels, selectedModelId]
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

  // Auto-scroll to bottom when new events arrive
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [chatEvents]);

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
      return sendUserMessageWithImages(org!, agentId, sessionId, text, images, controls);
    },
  });

  // Check if can submit (has content and all images uploaded)
  const hasContent = inputValue.trim() || hasImages;
  const canSubmit = hasContent && allUploaded && !sendMessage.isPending && !sendMessageWithImages.isPending;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;

    // Build controls with reasoning effort if selected
    const controls: Controls | undefined =
      selectedModelId || reasoningEffort
        ? {
            ...(selectedModelId && { model_id: selectedModelId }),
            ...(reasoningEffort && supportsReasoning && {
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
          agentId,
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
  const getMessageImages = (content: ContentPart[]): Array<{ image_id: string; filename?: string }> => {
    return content.filter(isImageFilePart).map((part) => ({
      image_id: part.image_id,
      filename: part.filename,
    }));
  };

  return (
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
            const images = data.message?.content ? getMessageImages(data.message.content) : [];

            return (
              <div key={event.id} className="space-y-2">
                {/* Render text content and images */}
                {(textContent || images.length > 0) && (
                  <div className={`flex ${isUser ? "justify-end" : "justify-start"}`}>
                    {isUser ? (
                      /* User message - dark box, 90% width */
                      <div className="max-w-[90%] bg-gray-500 text-white rounded-lg p-3">
                        <div className="flex items-start gap-2">
                          <div className="flex-1 space-y-2">
                            {textContent && (
                              <p className="text-sm whitespace-pre-wrap">{textContent}</p>
                            )}
                            {images.length > 0 && (
                              <div className="flex flex-wrap gap-2 mt-2">
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
                          <MessageInfoIcon event={event} variant="light" />
                        </div>
                      </div>
                    ) : (
                      /* Agent message - darker background with robot icon */
                      <div className="w-full bg-muted/60 rounded-lg p-3">
                        <div className="flex items-start gap-2">
                          <Bot className="w-4 h-4 mt-0.5 flex-shrink-0 text-muted-foreground" />
                          <div className="flex-1 space-y-2">
                            {textContent && (
                              <p className="text-sm whitespace-pre-wrap">{textContent}</p>
                            )}
                            {images.length > 0 && (
                              <div className="flex flex-wrap gap-2 mt-2">
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

                {/* Render tool calls from agent message */}
                {toolCalls.length > 0 && (
                  <div className="pl-[25px] space-y-2">
                    {toolCalls.map((tc) => {
                      const toolResult = toolResultsMap.get(tc.id);
                      return (
                        <ToolCallCardFromEvent key={tc.id} toolCall={tc} toolResult={toolResult} />
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
        {/* Image attachments preview */}
        {hasImages && (
          <div className="mb-2">
            <ImageAttachments images={pendingImages} onRemove={removeImage} />
          </div>
        )}

        <form onSubmit={handleSubmit} className="flex gap-2">
          {/* Hidden file input */}
          <input
            ref={fileInputRef}
            type="file"
            accept={ALLOWED_IMAGE_TYPES.join(",")}
            multiple
            className="hidden"
            onChange={handleFileChange}
          />

          {/* Image attachment button */}
          <Button
            type="button"
            variant="outline"
            size="icon"
            className="h-[60px] w-[60px] flex-shrink-0"
            onClick={() => fileInputRef.current?.click()}
            title="Attach images (PNG, JPEG, GIF, WebP)"
          >
            <ImagePlus className="h-5 w-5" />
          </Button>

          {/* Textarea with drag-drop wrapper */}
          <div
            className={`flex-1 relative rounded-md transition-colors ${
              isDraggingOver
                ? "bg-primary/10 ring-2 ring-primary/50 ring-offset-2"
                : ""
            }`}
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
              // Only set to false if leaving the container (not entering a child)
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
                const imageFiles = Array.from(files).filter((f) =>
                  f.type.startsWith("image/")
                );
                if (imageFiles.length > 0) {
                  addFiles(imageFiles);
                }
              }
            }}
          >
            <Textarea
              ref={textareaRef}
              value={inputValue}
              onChange={(e) => setInputValue(e.target.value)}
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
              placeholder="Type a message... (Paste or drop images, Enter to send)"
              className="w-full min-h-[60px] max-h-[200px] resize-none"
            />
          </div>

          <Button
            type="submit"
            size="icon"
            className="h-[60px] w-[60px]"
            disabled={!canSubmit}
            title={isUploading ? "Uploading images..." : undefined}
          >
            {sendMessage.isPending || sendMessageWithImages.isPending ? (
              <Loader2 className="h-5 w-5 animate-spin" />
            ) : isUploading ? (
              <Loader2 className="h-5 w-5 animate-spin" />
            ) : (
              <Send className="h-5 w-5" />
            )}
          </Button>
        </form>
        <div className="flex flex-wrap items-center gap-4 mt-2">
          <div className="flex items-center gap-2">
            <span className="text-sm text-muted-foreground">Model:</span>
            <Select
              value={selectedModelId}
              onValueChange={(value) => {
                hasUserSelectedModel.current = true;
                setSelectedModelId(value);
              }}
            >
              <SelectTrigger size="sm" className="w-[220px]">
                <SelectValue>
                  {selectedModelId
                    ? selectedModel?.display_name ?? "Select model"
                    : llmModel?.display_name
                      ? `Session default (${llmModel.display_name})`
                      : "Session default"}
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="">
                  {llmModel?.display_name
                    ? `Session default (${llmModel.display_name})`
                    : "Session default"}
                </SelectItem>
                {llmModels.map((model) => (
                  <SelectItem key={model.id} value={model.id}>
                    {model.display_name}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          {supportsReasoning && reasoningEffortConfig && (
            <div className="flex items-center gap-2">
              <Brain className="h-4 w-4 text-muted-foreground" />
              <span className="text-sm text-muted-foreground">Reasoning:</span>
              <Select
                value={reasoningEffort}
                onValueChange={(value) => setReasoningEffort(value as typeof reasoningEffort)}
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
        </div>
      </div>
    </>
  );
}
