"use client";

import { DevPageShell } from "@/app/dev/_components/dev-page-shell";
import {
  bashToolCall,
  bashToolOutput,
  bashToolResult,
  groupedActivityProgress,
  groupedActivityResults,
  groupedActivityToolCalls,
  readFileToolCall,
  readFileToolResult,
  timelineRows,
  todoToolCall,
  todoToolResult,
  writeFileToolCall,
  writeFileToolResult,
} from "@/app/dev/_fixtures/tool-showcase-fixtures";
import { BashToolCallCard } from "@/components/chat/bash-tool-call-card";
import { GroupedActivityCard } from "@/components/chat/grouped-activity-card";
import { ReadFileToolCallCard } from "@/components/chat/read-file-tool-call-card";
import { TodoListRenderer } from "@/components/chat/todo-list-renderer";
import { ToolActivityGroup } from "@/components/chat/tool-activity-group";
import { ToolActivityTimelineGroup } from "@/components/chat/tool-activity-timeline-group";
import { WriteFileToolCallCard } from "@/components/chat/write-file-tool-call-card";

export default function ToolActivityDevPage() {
  return (
    <DevPageShell
      eyebrow="Tool Outputs"
      title="Tool Outputs"
      description="A section-per-output reference page built entirely from the production tool transcript components."
      widthClassName="max-w-7xl"
    >
      <div className="space-y-6">
        <section className="space-y-4 border border-border/70 bg-card/90 p-4">
          <h2 className="text-lg font-semibold text-foreground">Conversational approval</h2>
          <div className="space-y-3">
            <p className="text-sm text-foreground">
              I can create Coding Agent with the Worker harness and Daytona sandbox. Shall I
              proceed?
            </p>
            <ToolActivityGroup
              toolCalls={[
                {
                  id: "approval-request-preview",
                  name: "request_approval",
                  arguments: {
                    action:
                      "Create the organisation-wide Coding Agent using the existing Worker harness and a Daytona primary sandbox configuration",
                    question:
                      "Shall I create Coding Agent for implementing, debugging and testing code, with approval required before pushes, deployments or destructive actions?",
                  },
                },
              ]}
              toolResultsMap={new Map()}
            />
          </div>
          <p id="consent-preview" className="text-right text-sm text-foreground">
            Approved
          </p>
          <ToolActivityGroup
            toolCalls={[
              {
                id: "approval-record-preview",
                name: "record_approval",
                arguments: {
                  action: "Create Coding Agent with the Worker harness and Daytona sandbox",
                },
              },
            ]}
            toolResultsMap={
              new Map([
                [
                  "approval-record-preview",
                  {
                    tool_call_id: "approval-record-preview",
                    tool_name: "record_approval",
                    success: true,
                    status: "success",
                  },
                ],
              ])
            }
            approvalContexts={
              new Map([
                [
                  "approval-record-preview",
                  {
                    approvedBy: "Avery Admin",
                    consentMessageHref: "#consent-preview",
                  },
                ],
              ])
            }
          />
        </section>
        <section className="space-y-4 border border-border/70 bg-card/90 p-4 shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]">
          <div className="space-y-1">
            <h2 className="text-lg font-semibold text-foreground">Standalone tool outputs</h2>
            <p className="text-sm text-muted-foreground">
              Bash, file reads, file writes, and todo plans as they appear in the transcript.
            </p>
          </div>

          <div className="grid min-w-0 gap-4 xl:grid-cols-2 [&>*]:min-w-0">
            <div className="space-y-2">
              <p className="text-sm font-medium text-foreground">Bash output</p>
              <BashToolCallCard
                toolCall={bashToolCall}
                toolResult={bashToolResult}
                streamedOutput={bashToolOutput}
              />
            </div>

            <div className="space-y-2">
              <p className="text-sm font-medium text-foreground">Read file output</p>
              <ReadFileToolCallCard toolCall={readFileToolCall} toolResult={readFileToolResult} />
            </div>

            <div className="space-y-2">
              <p className="text-sm font-medium text-foreground">Write file output</p>
              <WriteFileToolCallCard
                toolCall={writeFileToolCall}
                toolResult={writeFileToolResult}
              />
            </div>

            <div className="space-y-2">
              <p className="text-sm font-medium text-foreground">Todo plan output</p>
              <TodoListRenderer
                arguments={todoToolCall.arguments}
                result={todoToolResult.result}
                isExecuting={false}
              />
            </div>
          </div>
        </section>

        <section className="space-y-4 border border-border/70 bg-card/90 p-4 shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]">
          <div className="space-y-1">
            <h2 className="text-lg font-semibold text-foreground">Grouped activity</h2>
            <p className="text-sm text-muted-foreground">
              The shared multi-tool execution card used when a turn batches several non-shell tool
              calls.
            </p>
          </div>

          <GroupedActivityCard
            toolCalls={groupedActivityToolCalls}
            toolResultsMap={groupedActivityResults}
            toolProgressMap={groupedActivityProgress}
            mode="server"
          />
        </section>

        <section className="space-y-4 border border-border/70 bg-card/90 p-4 shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]">
          <div className="space-y-1">
            <h2 className="text-lg font-semibold text-foreground">Narrated tool timeline</h2>
            <p className="text-sm text-muted-foreground">
              The Daytona-style narrated activity block that tracks an execution across several tool
              steps.
            </p>
          </div>

          <ToolActivityTimelineGroup
            headline="Running Daytona sandbox checks"
            completedHeadline="Ran Daytona sandbox checks"
            rows={timelineRows}
          />
        </section>

        <section className="space-y-4 border border-border/70 bg-card/90 p-4 shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]">
          <div className="space-y-1">
            <h2 className="text-lg font-semibold text-foreground">Combined transcript grouping</h2>
            <p className="text-sm text-muted-foreground">
              The router component that decides when to render standalone rows, grouped activity, or
              todo cards.
            </p>
          </div>

          <ToolActivityGroup
            toolCalls={[bashToolCall, ...groupedActivityToolCalls, todoToolCall]}
            toolResultsMap={
              new Map([
                [bashToolCall.id, bashToolResult],
                ...groupedActivityResults,
                [todoToolCall.id, todoToolResult],
              ])
            }
            toolProgressMap={groupedActivityProgress}
            toolOutputMap={new Map([[bashToolCall.id, bashToolOutput]])}
            mode="server"
          />
        </section>
      </div>
    </DevPageShell>
  );
}
