import { buildToolActivityGroups, hostedToolLabel } from "@/components/chat/tool-activity-groups";
import type { Event } from "@/lib/api/types";

function event(
  id: string,
  type: Event["type"],
  execId: string | undefined,
  data: Event["data"],
  sequence: number,
): Event {
  return {
    id,
    type,
    ts: `2026-08-08T00:00:${String(sequence).padStart(2, "0")}Z`,
    sequence,
    session_id: "session-test",
    context: { turn_id: "turn-test", exec_id: execId },
    data,
  };
}

const summary = (id: string, name = "discover") => ({
  id,
  name,
  narration: `Running ${name}`,
  completed_narration: `Ran ${name}`,
});

describe("buildToolActivityGroups", () => {
  it("folds one act lifecycle into one group", () => {
    const events = [
      event(
        "start",
        "act.started",
        "exec-1",
        { headline: "Running discover", tool_calls: [summary("call-1")] },
        1,
      ),
      event(
        "tool",
        "tool.completed",
        "exec-1",
        {
          tool_call_id: "call-1",
          tool_name: "discover",
          success: true,
          status: "success",
          narration: "Ran discover",
        },
        2,
      ),
      event(
        "done",
        "act.completed",
        "exec-1",
        { completed: true, success_count: 1, error_count: 0, headline: "Ran discover" },
        3,
      ),
    ];

    const built = buildToolActivityGroups(events, "Working");
    expect([...built.byAnchorEventId]).toHaveLength(1);
    expect(built.byAnchorEventId.get("start")?.rows).toHaveLength(1);
    expect(built.byAnchorEventId.get("start")?.completedHeadline).toBe("Ran discover");
  });

  it("deduplicates replayed events and reconciles same-exec retries", () => {
    const start = event(
      "start",
      "act.started",
      "exec-1",
      { headline: "Running", tool_calls: [summary("call-1")] },
      1,
    );
    const retry = event(
      "retry-start",
      "act.started",
      "exec-1",
      { headline: "Running", tool_calls: [summary("call-1")] },
      2,
    );
    const done = event(
      "done",
      "tool.completed",
      "exec-1",
      { tool_call_id: "call-1", tool_name: "discover", success: true, status: "success" },
      3,
    );
    const built = buildToolActivityGroups([start, start, retry, done], "Working");

    expect([...built.byAnchorEventId]).toHaveLength(1);
    expect(built.byAnchorEventId.get("start")?.rows).toHaveLength(1);
    expect(built.byAnchorEventId.get("start")?.rows[0].state).toBe("completed");
  });

  it("preserves distinct repeated calls and distinct exec batches", () => {
    const built = buildToolActivityGroups(
      [
        event(
          "start-1",
          "act.started",
          "exec-1",
          {
            headline: "Discovered operations twice",
            tool_calls: [summary("call-1"), summary("call-2")],
          },
          1,
        ),
        event(
          "start-2",
          "act.started",
          "exec-2",
          { headline: "Discovering operations", tool_calls: [summary("call-3")] },
          2,
        ),
      ],
      "Working",
    );

    expect([...built.byAnchorEventId]).toHaveLength(2);
    expect(built.byAnchorEventId.get("start-1")?.rows.map((row) => row.id)).toEqual([
      "call-1",
      "call-2",
    ]);
  });

  it("merges requested calls and client results by exec and call id", () => {
    const built = buildToolActivityGroups(
      [
        event(
          "start",
          "act.started",
          "exec-1",
          { headline: "Running server tool", tool_calls: [summary("server", "query")] },
          1,
        ),
        event(
          "request",
          "tool.call_requested",
          "exec-1",
          {
            headline: "Waiting on browser",
            completed_headline: "Used browser",
            tool_calls: [{ id: "client", name: "browser", arguments: {} }],
            tool_summaries: [summary("client", "browser")],
          },
          2,
        ),
        event(
          "client-done",
          "tool.completed",
          undefined,
          { tool_call_id: "client", tool_name: "", success: true, status: "success" },
          3,
        ),
      ],
      "Working",
    );

    expect([...built.byAnchorEventId]).toHaveLength(1);
    expect(built.byAnchorEventId.get("start")?.rows).toHaveLength(2);
    expect(built.byAnchorEventId.get("start")?.rows[1]).toMatchObject({
      id: "client",
      state: "completed",
      label: "Ran browser",
    });
  });

  it("builds an orphan completion group until pagination supplies its start", () => {
    const completion = event(
      "done",
      "tool.completed",
      "exec-1",
      {
        tool_call_id: "call-1",
        tool_name: "query",
        success: true,
        status: "success",
        narration: "Queried platform",
      },
      2,
    );
    const partial = buildToolActivityGroups([completion], "Working");
    expect(partial.byAnchorEventId.get("done")?.rows[0].state).toBe("completed");

    const enriched = buildToolActivityGroups(
      [
        event(
          "start",
          "act.started",
          "exec-1",
          { headline: "Querying platform", tool_calls: [summary("call-1", "query")] },
          1,
        ),
        completion,
      ],
      "Working",
    );
    expect(enriched.byAnchorEventId.has("start")).toBe(true);
    expect(enriched.byAnchorEventId.get("start")?.rows[0].state).toBe("completed");
  });

  it("folds live progress without creating another group", () => {
    const built = buildToolActivityGroups(
      [
        event(
          "start",
          "act.started",
          "exec-1",
          { headline: "Searching", tool_calls: [summary("call-1", "search_web")] },
          1,
        ),
        event(
          "progress",
          "tool.progress",
          "exec-1",
          { tool_call_id: "call-1", tool_name: "search_web", message: "Reading result 2 of 3" },
          2,
        ),
      ],
      "Working",
    );
    expect([...built.byAnchorEventId]).toHaveLength(1);
    expect(built.byAnchorEventId.get("start")?.rows[0].label).toBe("Reading result 2 of 3");
  });

  it("leaves setup_connection requests to the specialized interactive card", () => {
    const request = event(
      "request",
      "tool.call_requested",
      "exec-1",
      {
        headline: "Waiting for connection",
        tool_calls: [{ id: "setup", name: "setup_connection", arguments: { provider: "daytona" } }],
        tool_summaries: [summary("setup", "setup_connection")],
      },
      1,
    );
    const built = buildToolActivityGroups([request], "Working");
    expect(built.byAnchorEventId.size).toBe(0);
    expect(built.groupedEventIds.has("request")).toBe(false);
  });

  it("keeps a mixed request renderable for its specialized card", () => {
    const request = event(
      "request",
      "tool.call_requested",
      "exec-1",
      {
        headline: "Waiting on tools",
        tool_calls: [
          { id: "client", name: "browser", arguments: {} },
          { id: "setup", name: "setup_connection", arguments: { provider: "daytona" } },
        ],
        tool_summaries: [summary("client", "browser"), summary("setup", "setup_connection")],
      },
      1,
    );
    const built = buildToolActivityGroups([request], "Working");
    expect(built.byAnchorEventId.get("request")?.rows.map((row) => row.id)).toEqual(["client"]);
    expect(built.groupedEventIds.has("request")).toBe(false);
  });

  it("folds hosted web search calls into one row per call", () => {
    const hosted = (id: string, status: string, summary: string | undefined, seq: number) =>
      event(
        id,
        "tool.hosted_call",
        undefined,
        { turn_id: "turn-test", call_id: "ws_1", tool_name: "web_search", status, summary },
        seq,
      );
    const built = buildToolActivityGroups(
      [hosted("h1", "in_progress", undefined, 1), hosted("h2", "completed", "everruns news", 2)],
      "Working",
    );
    const group = built.byAnchorEventId.get("h1");
    expect(group?.rows).toEqual([
      { id: "ws_1", label: "Search web for everruns news", state: "completed" },
    ]);
    expect(built.groupedEventIds.has("h1")).toBe(true);
    expect(built.groupedEventIds.has("h2")).toBe(true);
  });

  it("marks a failed hosted call and localizes its label", () => {
    const failed = event(
      "h1",
      "tool.hosted_call",
      undefined,
      { turn_id: "turn-test", call_id: "ws_2", tool_name: "web_search", status: "failed" },
      1,
    );
    const row = buildToolActivityGroups([failed], "Працюю", "uk").byAnchorEventId.get("h1")
      ?.rows[0];
    expect(row?.state).toBe("error");
    expect(row?.label).toBe("Пошук у вебі");
  });

  it("labels every OpenAI hosted tool from its detail", () => {
    const label = (tool_name: string, summary?: string, locale = "en") =>
      hostedToolLabel(
        { turn_id: "t", call_id: "c", tool_name, status: "completed", summary },
        locale,
      );
    expect(label("code_interpreter", "2**100")).toBe("Run code: 2**100");
    expect(label("code_interpreter", undefined, "uk")).toBe("Виконати код");
    expect(label("shell", "uname -s")).toBe("$ uname -s");
    expect(label("shell")).toBe("Shell");
    expect(label("file_search", "refund policy")).toBe("Find refund policy");
    expect(label("file_search")).toBe("Search files");
    expect(label("image_generation")).toBe("Image Generation");
  });

  it("leaves approval lifecycle entries to their dedicated chat cards", () => {
    const approvalSummary = {
      id: "approval-1",
      name: "record_approval",
      narration: "Recording approval",
      completed_narration: "Approval recorded",
    };
    const built = buildToolActivityGroups(
      [
        event(
          "start",
          "act.started",
          "exec-1",
          { headline: "Recording approval", tool_calls: [approvalSummary] },
          1,
        ),
        event(
          "complete",
          "tool.completed",
          "exec-1",
          {
            tool_call_id: "approval-1",
            tool_name: "record_approval",
            success: true,
            status: "success",
            narration: "Approval recorded",
          },
          2,
        ),
      ],
      "Working",
    );

    expect(built.byAnchorEventId.size).toBe(0);
    expect(built.narratedToolCallIds.has("approval-1")).toBe(false);
  });
});
