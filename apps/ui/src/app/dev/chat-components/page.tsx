"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { useMemo, useRef, useState } from "react";
import { DevPageShell } from "@/app/dev/_components/dev-page-shell";
import { DevChatRuntimeScene } from "@/app/dev/_components/dev-chat-runtime-preview";
import { DevChatNavRailPreview } from "@/app/dev/_components/dev-chat-nav-rail-preview";
import {
  devCommands,
  devModels,
  makeInputEvent,
  makeOutputEvent,
  makePendingImages,
} from "@/app/dev/_fixtures/chat-runtime-fixtures";
import { ChatComposer } from "@/components/chat/chat-composer";
import { getTextFromContent, getToolCallsFromContent } from "@/lib/api/types";
import { NoIntelligenceMessage } from "@/components/chat/no-intelligence-notice";
import { ChatMessageList } from "@/components/chat/chat-message-list";
import { chatSurfaceStyles } from "@/components/chat/chat-surface";
import type { PendingImage } from "@/lib/api/images";
import type { ReasoningEffortConfig, SessionTask, VerbosityConfig } from "@/lib/api/types";
import type { ChatRun } from "@/components/chat/run-cards";

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
  const ui = useUiTranslations();
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [inputValue, setInputValue] = useState("/ship tighten the tool transcript UI");
  const [selectedModelId, setSelectedModelId] = useState(devModels[0]?.id ?? "");
  const [reasoningEffort, setReasoningEffort] = useState("medium");
  const [verbosity, setVerbosity] = useState("");
  const [pendingImages, setPendingImages] = useState<PendingImage[]>(() => makePendingImages());

  const emptyToolResults = useMemo(() => new Map(), []);
  const emptyToolProgress = useMemo(() => new Map(), []);
  const emptyToolOutputs = useMemo(() => new Map(), []);
  const automaticUpdateEvents = useMemo(() => {
    const sessionId = "session-automatic-updates";
    const wake = (id: string, sequence: number, text: string) =>
      makeInputEvent({
        id,
        sequence,
        sessionId,
        text,
        ts: `2026-10-26T02:2${sequence}:00Z`,
        metadata: { everruns_origin: "task_wake" },
      });
    const reply = (id: string, sequence: number, text: string) =>
      makeOutputEvent({
        id,
        sequence,
        sessionId,
        text,
        ts: `2026-10-26T02:2${sequence}:00Z`,
      });
    return [
      reply("reply-start", 1, "Started a thread to investigate the three sandboxes."),
      wake(
        "wake-adjacent",
        2,
        'Task "Investigate three running sandboxes" (task_00sbx) finished: succeeded.\n- summary: All three records show Running, but Daytona returns not_found for each.',
      ),
      reply(
        "reply-cleanup",
        3,
        "All three sandboxes are gone on Daytona's side. I started a cleanup proposal.",
      ),
      makeInputEvent({
        id: "user-quota",
        sequence: 4,
        sessionId,
        text: "Meanwhile, what's our Daytona quota?",
        ts: "2026-10-26T02:24:00Z",
      }),
      reply("reply-quota", 5, "You have 20 sandboxes, 3 in use."),
      wake(
        "wake-done",
        6,
        'Task "Prepare closed-PR sandbox cleanup" (task_01abc) finished: succeeded.\n- summary: PROPOSAL READY, NOT ENABLED. All closed pull requests, seven days after closure.',
      ),
      reply(
        "reply-done",
        7,
        "Prepare closed-PR sandbox cleanup is ready as a proposal. Nothing is enabled yet.",
      ),
      wake("wake-progress", 8, 'Task "Pricing FAQ" (task_02def) sent a message: halfway'),
      wake(
        "wake-ask",
        9,
        'Task "Pricing FAQ" (task_02def) is awaiting input: Which tone, plain or formal?',
      ),
    ];
  }, []);

  const automaticUpdateRuns = useMemo(() => {
    const task = (id: string, display_name: string, started_at: string, finished_at: string) =>
      ({
        id,
        session_id: "session-automatic-updates",
        kind: "subagent",
        display_name,
        state: "succeeded",
        wake_policy: "on_terminal",
        created_at: started_at,
        updated_at: finished_at,
        started_at,
        finished_at,
      }) as unknown as SessionTask;
    return new Map<string, ChatRun[]>([
      [
        "reply-start",
        [
          {
            task: task(
              "task_00sbx",
              "Investigate three running sandboxes",
              "2026-10-26T02:20:00Z",
              "2026-10-26T02:21:45Z",
            ),
            subagentCount: 0,
          },
        ],
      ],
      [
        "reply-cleanup",
        [
          {
            task: task(
              "task_01abc",
              "Prepare closed-PR sandbox cleanup",
              "2026-10-26T02:23:00Z",
              "2026-10-26T02:25:30Z",
            ),
            subagentCount: 0,
          },
        ],
      ],
    ]);
  }, []);

  return (
    <DevPageShell
      eyebrow="Chat UI"
      title={ui("Chat UI")}
      description={ui(
        "Runtime chat surfaces driven by the real transcript and composer components.",
      )}
      widthClassName="max-w-7xl"
    >
      <div className="space-y-6">
        <section className="space-y-3">
          <div className="space-y-1">
            <h2 className="text-lg font-semibold text-foreground">
              {ui("Tool-heavy runtime scene")}
            </h2>
            <p className="text-sm text-muted-foreground">
              {ui(
                "Full session chrome plus the actual chat panel, using the Daytona-style transcript flow.",
              )}
            </p>
          </div>
          <DevChatRuntimeScene scenario="tool-activity" />
        </section>

        <section className="space-y-3">
          <div className="space-y-1">
            <h2 className="text-lg font-semibold text-foreground">
              {ui("Transcript-focused runtime scene")}
            </h2>
            <p className="text-sm text-muted-foreground">
              {ui("Same runtime stack, but tuned to text, todos, and file mutation output.")}
            </p>
          </div>
          <DevChatRuntimeScene scenario="chat-components" />
        </section>

        <section className="space-y-3">
          <div className="space-y-1">
            <h2 className="text-lg font-semibold text-foreground">{ui("Turn navigation rail")}</h2>
            <p className="text-sm text-muted-foreground">
              {ui(
                "Multi-turn transcript with the turn navigation rail (md+ screens). Hover a marker on the right gutter to preview the turn; click to jump. The active turn fills with the gold accent as you scroll.",
              )}
            </p>
          </div>
          <DevChatNavRailPreview />
        </section>

        <section className="space-y-3">
          <div className="space-y-1">
            <h2 className="text-lg font-semibold text-foreground">
              {ui("No intelligence available")}
            </h2>
            <p className="text-sm text-muted-foreground">
              {ui(
                "Shown on every chat surface when the org has no enabled, healthy chat model. It is always the only centred message on the surface: it replaces the transcript’s “No messages yet” card on a fresh thread, sits above the composer once there is history, and replaces the counterpart picker on the new-chat form. The route to Settings only appears for a caller who can manage providers.",
              )}
            </p>
          </div>
          <div className="grid gap-4 border border-border/70 bg-background p-6 md:grid-cols-2">
            <div className="space-y-2">
              <p className="text-xs font-medium uppercase tracking-[0.2em] text-muted-foreground">
                {ui("Can manage providers")}
              </p>
              <NoIntelligenceMessage canManage />
            </div>
            <div className="space-y-2">
              <p className="text-xs font-medium uppercase tracking-[0.2em] text-muted-foreground">
                {ui("Member, view only")}
              </p>
              <NoIntelligenceMessage canManage={false} />
            </div>
            <div className="space-y-2 md:col-span-2">
              <p className="text-xs font-medium uppercase tracking-[0.2em] text-muted-foreground">
                {ui("Plain, inside a host that draws its own frame")}
              </p>
              <NoIntelligenceMessage canManage variant="plain" />
            </div>
          </div>
        </section>

        <section className="space-y-3" data-testid="automatic-updates">
          <div className="space-y-1">
            <h2 className="text-lg font-semibold text-foreground">{ui("Automatic updates")}</h2>
            <p className="text-sm text-muted-foreground">
              {ui(
                "A task wake right after its run card adds no row. Further down it is one status line; the report stays in the task's thread.",
              )}
            </p>
          </div>
          <div className={chatSurfaceStyles.column}>
            <ChatMessageList
              events={automaticUpdateEvents}
              chatEvents={automaticUpdateEvents}
              runsByEventId={automaticUpdateRuns}
              sessionId="session-automatic-updates"
              toolResultsMap={emptyToolResults}
              toolProgressMap={emptyToolProgress}
              toolOutputMap={emptyToolOutputs}
              eventsLoading={false}
              hasMoreEvents={false}
              loadingOlderEvents={false}
              getMessageText={(data) => getTextFromContent(data.message?.content ?? [])}
              getToolCalls={(data) => getToolCallsFromContent(data.message?.content ?? [])}
            />
          </div>
        </section>

        <section className="space-y-3 border border-border/70 bg-card/90 p-4 shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]">
          <div className="space-y-1">
            <h2 className="text-lg font-semibold text-foreground">
              {ui("Empty state and composer")}
            </h2>
            <p className="text-sm text-muted-foreground">
              {ui(
                "Direct preview of the empty transcript state and the production composer controls.",
              )}
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
                  pendingFiles={[]}
                  hasFiles={false}
                  removeFileAttachment={() => undefined}
                  supportsPdf={false}
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
