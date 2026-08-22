"use client";

import { useMemo, useRef, useState } from "react";
import { DevPageShell } from "@/app/dev/_components/dev-page-shell";
import { DevChatRuntimeScene } from "@/app/dev/_components/dev-chat-runtime-preview";
import { DevChatNavRailPreview } from "@/app/dev/_components/dev-chat-nav-rail-preview";
import {
  devCommands,
  devModels,
  makePendingImages,
} from "@/app/dev/_fixtures/chat-runtime-fixtures";
import { ChatComposer } from "@/components/chat/chat-composer";
import { ChatMessageList } from "@/components/chat/chat-message-list";
import { chatSurfaceStyles } from "@/components/chat/chat-surface";
import type { PendingImage } from "@/lib/api/images";
import type { ReasoningEffortConfig, VerbosityConfig } from "@/lib/api/types";

const reasoningEffortConfig: ReasoningEffortConfig = {
  default: "medium",
  values: [
    { value: "low", name: "Low" },
    { value: "medium", name: "Medium" },
    { value: "high", name: "High" },
  ],
};

const verbosityConfig: VerbosityConfig = {
  default: "medium",
  values: [
    { value: "low", name: "Low" },
    { value: "medium", name: "Medium" },
    { value: "high", name: "High" },
  ],
};

export default function DevChatComponentsPage() {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [inputValue, setInputValue] = useState("/ship tighten the tool transcript UI");
  const [selectedModelId, setSelectedModelId] = useState(devModels[0]?.id ?? "");
  const [reasoningEffort, setReasoningEffort] = useState("medium");
  const [verbosity, setVerbosity] = useState("");
  const [pendingImages, setPendingImages] = useState<PendingImage[]>(() => makePendingImages());

  const emptyToolResults = useMemo(() => new Map(), []);
  const emptyToolProgress = useMemo(() => new Map(), []);
  const emptyToolOutputs = useMemo(() => new Map(), []);

  return (
    <DevPageShell
      eyebrow="Chat UI"
      title="Chat UI"
      description="Runtime chat surfaces driven by the real transcript and composer components."
      widthClassName="max-w-7xl"
    >
      <div className="space-y-6">
        <section className="space-y-3">
          <div className="space-y-1">
            <h2 className="text-lg font-semibold text-foreground">Tool-heavy runtime scene</h2>
            <p className="text-sm text-muted-foreground">
              Full session chrome plus the actual chat panel, using the Daytona-style transcript
              flow.
            </p>
          </div>
          <DevChatRuntimeScene scenario="tool-activity" />
        </section>

        <section className="space-y-3">
          <div className="space-y-1">
            <h2 className="text-lg font-semibold text-foreground">
              Transcript-focused runtime scene
            </h2>
            <p className="text-sm text-muted-foreground">
              Same runtime stack, but tuned to text, todos, and file mutation output.
            </p>
          </div>
          <DevChatRuntimeScene scenario="chat-components" />
        </section>

        <section className="space-y-3">
          <div className="space-y-1">
            <h2 className="text-lg font-semibold text-foreground">Turn navigation rail</h2>
            <p className="text-sm text-muted-foreground">
              Multi-turn transcript with the turn navigation rail (md+ screens). Hover a marker on
              the right gutter to preview the turn; click to jump. The active turn fills with the
              gold accent as you scroll.
            </p>
          </div>
          <DevChatNavRailPreview />
        </section>

        <section className="space-y-3 border border-border/70 bg-card/90 p-4 shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]">
          <div className="space-y-1">
            <h2 className="text-lg font-semibold text-foreground">Empty state and composer</h2>
            <p className="text-sm text-muted-foreground">
              Direct preview of the empty transcript state and the production composer controls.
            </p>
          </div>

          <div className="overflow-hidden border border-border/70 bg-background">
            <div className="flex min-h-[720px] flex-col">
              <div className="flex-1 px-4 py-4">
                <ChatMessageList
                  events={[]}
                  chatEvents={[]}
                  sessionId="session-chat-empty"
                  toolResultsMap={emptyToolResults}
                  toolProgressMap={emptyToolProgress}
                  toolOutputMap={emptyToolOutputs}
                  eventsLoading={false}
                  hasMoreEvents={false}
                  loadingOlderEvents={false}
                  getMessageText={() => ""}
                  getToolCalls={() => []}
                />
              </div>

              <div className={chatSurfaceStyles.composerSection}>
                <ChatComposer
                  commands={devCommands}
                  models={devModels}
                  inputValue={inputValue}
                  onInputChange={setInputValue}
                  onSubmit={async () => undefined}
                  onCommandSelect={async () => undefined}
                  pendingImages={pendingImages}
                  hasImages={pendingImages.length > 0}
                  removeImage={(tempId) =>
                    setPendingImages((current) =>
                      current.filter((image) => image.tempId !== tempId),
                    )
                  }
                  addFiles={() => undefined}
                  isDraggingOver={false}
                  dropZoneProps={{}}
                  handlePaste={() => undefined}
                  selectedModelId={selectedModelId}
                  recentModels={devModels.slice(0, 2)}
                  onModelChange={setSelectedModelId}
                  modelTriggerLabel={
                    devModels.find((model) => model.id === selectedModelId)?.display_name ??
                    "Default model"
                  }
                  defaultModelOptionLabel="Default model"
                  supportsReasoning
                  reasoningEffort={reasoningEffort}
                  reasoningEffortConfig={reasoningEffortConfig}
                  defaultEffortName="Medium"
                  getReasoningEffortName={(value) =>
                    reasoningEffortConfig.values.find((effort) => effort.value === value)?.name ??
                    value
                  }
                  onReasoningEffortChange={setReasoningEffort}
                  supportsVerbosity
                  verbosity={verbosity}
                  verbosityConfig={verbosityConfig}
                  defaultVerbosityName="Medium"
                  getVerbosityName={(value) =>
                    verbosityConfig.values.find((item) => item.value === value)?.name ?? value
                  }
                  onVerbosityChange={setVerbosity}
                  isActive={false}
                  cancelCurrentTurn={{
                    mutate: () => undefined,
                    isPending: false,
                  }}
                  canSubmit={inputValue.trim().length > 0 || pendingImages.length > 0}
                  modelReady={Boolean(selectedModelId)}
                  modelLoading={false}
                  isUploading={pendingImages.some((image) => image.status === "uploading")}
                  sendPending={false}
                  textareaRef={textareaRef}
                />
              </div>
            </div>
          </div>
        </section>
      </div>
    </DevPageShell>
  );
}
