"use client";

/**
 * Dev page: Turn work log
 *
 * The folded "Working" section in its three states: a live turn (collapsed,
 * with elapsed time, latest step, and issue count), the same turn opened, and
 * a completed turn. Timestamps are anchored to mount time so the live counter
 * ticks.
 */

import { useEffect, useState } from "react";
import type React from "react";
import type { Event, ToolCompletedData, ToolProgressData } from "@/lib/api/types";
import type { ToolOutputStreams } from "@/app/(main)/sessions/[sessionId]/session-context";
import { DevPageShell } from "@/app/dev/_components/dev-page-shell";
import { makeInputEvent, makeOutputEvent } from "@/app/dev/_fixtures/chat-runtime-fixtures";
import { ChatMessageList } from "@/components/chat/chat-message-list";
import { getTextFromContent, getToolCallsFromContent } from "@/lib/api/types";

const SESSION_ID = "session-dev-work-log";
const emptyToolResults = new Map<string, ToolCompletedData>();
const emptyToolProgress = new Map<string, ToolProgressData>();
const emptyToolOutputs = new Map<string, ToolOutputStreams>();

function act(
  turnId: string,
  key: string,
  ts: string,
  headline: string,
  toolName: string,
  outcome: "ok" | "error" | "running",
  error?: string,
): Event[] {
  const execId = `exec-${turnId}-${key}`;
  const toolCallId = `${turnId}-${key}`;
  const context = { turn_id: turnId, exec_id: execId };
  const events: Event[] = [
    {
      id: `${turnId}-${key}-act`,
      type: "act.started",
      ts,
      session_id: SESSION_ID,
      context,
      data: { headline, tool_calls: [{ id: toolCallId, name: toolName, narration: headline }] },
    },
  ];
  if (outcome === "running") return events;
  return [
    ...events,
    {
      id: `${turnId}-${key}-done`,
      type: "tool.completed",
      ts,
      session_id: SESSION_ID,
      context,
      data: {
        tool_call_id: toolCallId,
        tool_name: toolName,
        success: outcome === "ok",
        status: outcome === "ok" ? "success" : "error",
        narration: headline,
        result: [{ type: "text", text: outcome === "ok" ? "ok" : (error ?? "failed") }],
        ...(outcome === "error" ? { error, severity: "issue" as const } : {}),
      },
    },
  ];
}

function turnEvents(turnId: string, startMs: number, live: boolean): Event[] {
  const at = (seconds: number) => new Date(startMs + seconds * 1000).toISOString();
  const events: Event[] = [
    makeInputEvent({
      id: `${turnId}-input`,
      sequence: 1,
      sessionId: SESSION_ID,
      ts: at(0),
      text: "Create a support agent that answers FAQs and helps troubleshoot issues.",
    }),
    {
      id: `${turnId}-started`,
      type: "turn.started",
      ts: at(0),
      session_id: SESSION_ID,
      context: { turn_id: turnId },
      data: { turn_id: turnId, input_message_id: `msg-${turnId}-input` },
    },
    ...act(turnId, "schema", at(3), "Getting the agent-creation command schema", "describe", "ok"),
    ...act(
      turnId,
      "create-1",
      at(8),
      "Creating the approved support agent",
      "create_agent",
      "error",
      "Tool execution error: create_agent: schema error: required property 'system_prompt' is missing",
    ),
    ...act(
      turnId,
      "create-2",
      at(14),
      "Creating the approved support agent",
      "create_agent",
      "error",
      'Tool execution error: bad_request: invalid type: string "Help me troubleshoot an issue", expected struct ConversationStarter',
    ),
  ];
  if (live) {
    return [
      ...events,
      ...act(
        turnId,
        "create-3",
        at(20),
        "Creating the approved support agent",
        "create_agent",
        "running",
      ),
    ];
  }
  return [
    ...events,
    ...act(turnId, "create-3", at(20), "Creating the approved support agent", "create_agent", "ok"),
    ...act(turnId, "verify", at(26), "Verifying the new support agent", "get_agent", "ok"),
    makeOutputEvent({
      id: `${turnId}-commentary`,
      sequence: 2,
      sessionId: SESSION_ID,
      ts: at(30),
      text: "There isn't a registered GitHub MCP server yet. I'll add it with OAuth and use the calling user's connection.",
      phase: "commentary",
      phaseSource: "provider",
      context: { turn_id: turnId },
    }),
    makeOutputEvent({
      id: `${turnId}-answer`,
      sequence: 3,
      sessionId: SESSION_ID,
      ts: at(32),
      text: "Created and verified **Support Agent**. It answers FAQs and helps troubleshoot issues.",
      phase: "final_answer",
      phaseSource: "provider",
      context: { turn_id: turnId },
    }),
    {
      id: `${turnId}-completed`,
      type: "turn.completed",
      ts: at(33),
      session_id: SESSION_ID,
      context: { turn_id: turnId },
      data: { turn_id: turnId, duration_ms: 33000 },
    },
  ];
}

/** A live turn with `count` finished tool calls, every 50th one failed. */
function largeTurnEvents(startMs: number, count: number): Event[] {
  const turnId = "turn-large";
  const at = (seconds: number) => new Date(startMs + seconds * 1000).toISOString();
  const events: Event[] = [
    makeInputEvent({
      id: `${turnId}-input`,
      sequence: 1,
      sessionId: SESSION_ID,
      ts: at(0),
      text: `Refactor every module (${count} tool calls).`,
    }),
    {
      id: `${turnId}-started`,
      type: "turn.started",
      ts: at(0),
      session_id: SESSION_ID,
      context: { turn_id: turnId },
      data: { turn_id: turnId, input_message_id: `msg-${turnId}-input` },
    },
  ];
  for (let index = 0; index < count; index += 1) {
    events.push(
      ...act(
        turnId,
        `call-${index}`,
        at(1),
        `Editing module ${index + 1}`,
        "edit_file",
        index % 50 === 49 ? "error" : "ok",
        `Tool execution error: conflict in module ${index + 1}`,
      ),
    );
  }
  return events;
}

/** Large-turn harness: `?calls=N` renders N tool calls; the button appends one more. */
function LargeTurn({ startMs, count }: { startMs: number; count: number }) {
  const [events, setEvents] = useState(() => largeTurnEvents(startMs, count));
  const appendCall = () =>
    setEvents((current) => [
      ...current,
      ...act(
        "turn-large",
        `call-${current.length}`,
        new Date().toISOString(),
        `Editing module ${current.length}`,
        "edit_file",
        "ok",
      ),
    ]);

  return (
    <Transcript
      title={`Large turn (${count} tool calls)`}
      events={events}
      action={
        <button
          type="button"
          data-testid="append-tool-call"
          onClick={appendCall}
          className="border border-border bg-background px-2 py-1 text-xs text-muted-foreground hover:text-foreground"
        >
          Append tool call
        </button>
      }
    />
  );
}

function Transcript({
  title,
  events,
  action,
  collapseWorkLog = true,
}: {
  title: string;
  events: Event[];
  action?: React.ReactNode;
  collapseWorkLog?: boolean;
}) {
  return (
    <section className="space-y-3 border border-border/70 bg-card/90 p-4">
      <div className="flex items-center justify-between gap-3">
        <h2 className="text-lg font-semibold text-foreground">{title}</h2>
        {action}
      </div>
      <div className="border border-border/70 bg-background px-4 py-4">
        <ChatMessageList
          events={events}
          collapseWorkLog={collapseWorkLog}
          chatEvents={events}
          sessionId={SESSION_ID}
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
  );
}

export default function DevWorkLogPage() {
  // Anchor to mount time on the client only, so SSR and hydration agree.
  const [mountedAt, setMountedAt] = useState<number | null>(null);
  const [largeCount, setLargeCount] = useState(0);
  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- one-shot client clock
    setMountedAt(Date.now());
    const calls = Number(new URLSearchParams(window.location.search).get("calls"));
    if (Number.isFinite(calls) && calls > 0) setLargeCount(Math.min(calls, 20000));
  }, []);

  return (
    <DevPageShell
      eyebrow="Chat"
      title="Turn work log"
      description="Platform Chat and Chats fold work into a compact section. Playground and Sessions keep the full log visible during and after a turn."
      widthClassName="max-w-4xl"
    >
      {mountedAt != null && (
        <div className="space-y-6">
          <Transcript title="Live turn" events={turnEvents("turn-live", mountedAt - 21000, true)} />
          <Transcript
            title="Completed turn"
            events={turnEvents("turn-done", mountedAt - 120000, false)}
          />
          <Transcript
            title="Playground / Sessions: live full log"
            events={turnEvents("turn-full-live", mountedAt - 21000, true)}
            collapseWorkLog={false}
          />
          <Transcript
            title="Playground / Sessions: completed full log"
            events={turnEvents("turn-full-done", mountedAt - 120000, false)}
            collapseWorkLog={false}
          />
          {largeCount > 0 && <LargeTurn startMs={mountedAt - 300000} count={largeCount} />}
        </div>
      )}
    </DevPageShell>
  );
}
