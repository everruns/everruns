"use client";

import { useUiTranslations } from "@/providers/locale-provider";
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
import { fallbackDetails } from "@/app/dev/_fixtures/web-search-fixtures";
import { ToolApprovalRequests } from "@/components/chat/tool-approval-tool-call";
import { McpApprovalToolCall } from "@/components/chat/mcp-approval-tool-call";
import { BashToolCallCard } from "@/components/chat/bash-tool-call-card";
import { GroupedActivityCard } from "@/components/chat/grouped-activity-card";
import { ReadFileToolCallCard } from "@/components/chat/read-file-tool-call-card";
import { TodoListRenderer } from "@/components/chat/todo-list-renderer";
import { ToolActivityGroup } from "@/components/chat/tool-activity-group";
import { ToolActivityTimelineGroup } from "@/components/chat/tool-activity-timeline-group";
import { WebSearchRow } from "@/components/chat/web-search-details";
import { WriteFileToolCallCard } from "@/components/chat/write-file-tool-call-card";
import { parseWebSearchDetails } from "@/lib/web-search";

const webSearchDetails = parseWebSearchDetails(fallbackDetails);

export default function ToolActivityDevPage() {
  const ui = useUiTranslations();
  return (
    <DevPageShell
      eyebrow="Tool Outputs"
      title={ui("Tool Outputs")}
      description={ui(
        "A section-per-output reference page built entirely from the production tool transcript components.",
      )}
      widthClassName="max-w-7xl"
    >
      <div className="space-y-6">
        <section aria-label="Approval control characters" className="max-w-3xl space-y-3">
          <h2 className="text-lg font-semibold text-foreground">Approval control characters</h2>
          <ToolApprovalRequests
            sessionId="approval-display-preview"
            requests={[
              {
                id: "tool_approval_display_preview",
                arguments: {
                  tool: "delete_file",
                  display_name: "Delete file",
                  risk: "destructive",
                  arguments: {
                    path: "reports/quarterly\u202Etxt.exe",
                    note: "می‌خواهم 👩‍💻",
                    literal: "\\u202E",
                  },
                },
              },
            ]}
            toolResultsMap={new Map()}
          />
          <McpApprovalToolCall
            sessionId="approval-display-preview"
            toolCallId="mcp_approval_display_preview"
            approval={{
              server_label: "Documents",
              name: "read\u2066file",
              arguments: JSON.stringify({ path: "reports/quarterly\u202Etxt.exe" }),
            }}
            toolResultsMap={new Map()}
          />
        </section>
        <section className="space-y-4 border border-border/70 bg-card/90 p-4">
          <h2 className="text-lg font-semibold text-foreground">{ui("Conversational approval")}</h2>
          <div className="space-y-3">
            <p className="text-sm text-foreground">
              {ui(
                "I can create Coding Agent with the Worker harness and Daytona sandbox. Shall I proceed?",
              )}
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
            {ui("Approved")}
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
            <h2 className="text-lg font-semibold text-foreground">
              {ui("Standalone tool outputs")}
            </h2>
            <p className="text-sm text-muted-foreground">
              {ui(
                "Bash, file reads, file writes, and todo plans as they appear in the transcript.",
              )}
            </p>
          </div>

          <div className="grid min-w-0 gap-4 xl:grid-cols-2 [&>*]:min-w-0">
            <div className="space-y-2">
              <p className="text-sm font-medium text-foreground">{ui("Bash output")}</p>
              <BashToolCallCard
                toolCall={bashToolCall}
                toolResult={bashToolResult}
                streamedOutput={bashToolOutput}
              />
            </div>

            <div className="space-y-2">
              <p className="text-sm font-medium text-foreground">{ui("Read file output")}</p>
              <ReadFileToolCallCard toolCall={readFileToolCall} toolResult={readFileToolResult} />
            </div>

            <div className="space-y-2">
              <p className="text-sm font-medium text-foreground">{ui("Write file output")}</p>
              <WriteFileToolCallCard
                toolCall={writeFileToolCall}
                toolResult={writeFileToolResult}
              />
            </div>

            <div className="space-y-2">
              <p className="text-sm font-medium text-foreground">{ui("Todo plan output")}</p>
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
            <h2 className="text-lg font-semibold text-foreground">{ui("Grouped activity")}</h2>
            <p className="text-sm text-muted-foreground">
              {ui(
                "The shared multi-tool execution card used when a turn batches several non-shell tool calls.",
              )}
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
            <h2 className="text-lg font-semibold text-foreground">
              {ui("Narrated tool timeline")}
            </h2>
            <p className="text-sm text-muted-foreground">
              {ui(
                "The Daytona-style narrated activity block that tracks an execution across several tool steps.",
              )}
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
            <h2 className="text-lg font-semibold text-foreground">{ui("Web search")}</h2>
            <p className="text-sm text-muted-foreground">
              {ui(
                'The "Searched" row for a web_search call that fell back: model provider search skipped, Brave Search rate limited, free search answered.',
              )}
            </p>
          </div>

          {webSearchDetails && <WebSearchRow details={webSearchDetails} durationMs={1300} />}
        </section>

        <section className="space-y-4 border border-border/70 bg-card/90 p-4 shadow-[inset_0_1px_0_hsl(var(--background)/0.92)]">
          <div className="space-y-1">
            <h2 className="text-lg font-semibold text-foreground">
              {ui("Combined transcript grouping")}
            </h2>
            <p className="text-sm text-muted-foreground">
              {ui(
                "The router component that decides when to render standalone rows, grouped activity, or todo cards.",
              )}
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
