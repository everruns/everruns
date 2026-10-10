import type { TraceEventRef, TraceItem, TraceStep, TraceTurn } from "@/lib/api/types";
import {
  batchKey,
  buildRows,
  fillGap,
  formatDuration,
  lifecycleEvents,
  mergeTurns,
  parseStepKey,
  rowIndexOfStep,
  searchSnippet,
  selectableKeys,
  stepHoldingSequence,
  stepRowKey,
  stepVisible,
  turnHoldingSequence,
  waterfallBar,
} from "@/components/session/trace/trace-model";

function step(n: number, kind: string, extra: Partial<TraceStep> = {}): TraceItem {
  return {
    type: "step",
    turn: 1,
    step: n,
    kind,
    status: "success",
    started_at: "2026-10-10T00:00:00Z",
    offset_ms: n * 100,
    duration_ms: 50,
    start_sequence: n,
    ...extra,
  };
}

function turn(n: number, items: TraceItem[]): TraceTurn {
  return {
    turn: n,
    turn_id: `turn_${n}`,
    status: "completed",
    started_at: "2026-10-10T00:00:00Z",
    duration_ms: 1000,
    step_count: items.length,
    model_calls: 1,
    tool_calls: 1,
    subagent_calls: 0,
    error_count: 0,
    input_tokens: 10,
    output_tokens: 5,
    start_sequence: 1,
    items,
  };
}

const items: TraceItem[] = [
  step(1, "model", { narration: "Looking it up." }),
  step(2, "tool", { status: "error", result: "404" }),
  {
    type: "batch",
    turn: 1,
    name: "web_fetch",
    first_step: 3,
    last_step: 9,
    count: 7,
    succeeded: 7,
    failed: 0,
    running: 0,
    started_at: "2026-10-10T00:00:00Z",
    offset_ms: 300,
    failures: [],
  },
  step(10, "answer", { narration: "Done." }),
];

describe("session trace model", () => {
  it("frames loaded turns with unloaded bars and turn headers and footers", () => {
    const rows = buildRows([turn(5, items)], 9, { view: "all", errorsOnly: false }, new Set());
    expect(rows.map((r) => r.type)).toEqual([
      "unloaded",
      "turn",
      "step",
      "step",
      "batch",
      "step",
      "footer",
      "unloaded",
    ]);
    expect(rows[0]).toMatchObject({ edge: "earlier", from: 1, to: 4 });
    expect(rows[7]).toMatchObject({ edge: "later", from: 6, to: 9 });
  });

  it("filters by view and errors", () => {
    const keep = (view: "all" | "messages" | "tools", errorsOnly = false) =>
      buildRows([turn(1, items)], 1, { view, errorsOnly }, new Set())
        .filter((r) => r.type === "step" || r.type === "batch")
        .map((r) => (r.type === "step" ? r.step.kind : "batch"));
    expect(keep("messages")).toEqual(["model", "answer"]);
    expect(keep("tools")).toEqual(["tool", "batch"]);
    expect(keep("all", true)).toEqual(["tool", "answer"]);
    expect(
      stepVisible(step(1, "model") as TraceStep, { view: "messages", errorsOnly: false }),
    ).toBe(false);
  });

  it("collapses a turn to its header and footer", () => {
    const rows = buildRows([turn(1, items)], 1, { view: "all", errorsOnly: false }, new Set([1]));
    expect(rows.map((r) => r.type)).toEqual(["turn"]);
  });

  it("says when a filter hides every step of a turn", () => {
    const rows = buildRows(
      [turn(1, [step(1, "model")])],
      1,
      { view: "tools", errorsOnly: false },
      new Set(),
    );
    expect(rows.map((r) => r.type)).toEqual(["turn", "empty", "footer"]);
  });

  it("merges pages in turn order, replacing repeats", () => {
    const merged = mergeTurns([turn(3, []), turn(4, [])], [turn(4, items), turn(2, [])]);
    expect(merged.map((t) => t.turn)).toEqual([2, 3, 4]);
    expect(merged[2]?.items).toHaveLength(items.length);
  });

  it("fills a gap from its start and keeps the rest as a smaller gap", () => {
    const gapped = turn(1, [
      step(1, "model"),
      { type: "gap", turn: 1, first_step: 2, last_step: 301, count: 300, errors: 2 },
    ]);
    const fetched = Array.from({ length: 100 }, (_, i) =>
      step(i + 2, "tool", i === 0 ? { status: "error" } : {}),
    ) as TraceStep[];
    const filled = fillGap(
      gapped,
      { turn: 1, first_step: 2, last_step: 301, count: 300, errors: 2 },
      fetched,
      102,
    );
    expect(filled.items).toHaveLength(1 + 100 + 1);
    expect(filled.items[101]).toMatchObject({
      type: "gap",
      first_step: 102,
      count: 200,
      errors: 1,
    });
    const done = fillGap(
      gapped,
      { turn: 1, first_step: 2, last_step: 301, count: 300, errors: 2 },
      fetched,
      null,
    );
    expect(done.items.some((i) => i.type === "gap")).toBe(false);
  });

  it("walks selectable steps and parses step keys", () => {
    const rows = buildRows([turn(1, items)], 1, { view: "all", errorsOnly: false }, new Set());
    expect(selectableKeys(rows)).toEqual(["1.1", "1.2", "1.10"]);
    expect(parseStepKey("12.7")).toEqual({ turn: 12, step: 7 });
    expect(parseStepKey("x")).toBeNull();
  });

  it("formats durations and places waterfall bars inside the turn", () => {
    expect(formatDuration(320)).toBe("320ms");
    expect(formatDuration(4200)).toBe("4.2s");
    expect(formatDuration(125_000)).toBe("2m 5s");
    expect(waterfallBar(500, 1000, 1000)).toEqual({ left: 0.5, width: 0.5 });
    expect(waterfallBar(2000, 10, 1000)).toEqual({ left: 1, width: 0 });
  });

  it("shows a batch's calls under it, paged and walked by j/k", () => {
    const calls = [3, 4, 5].map((n) =>
      step(n, "tool", n === 4 ? { status: "error" } : {}),
    ) as TraceStep[];
    const expanded = new Map([[batchKey(1, 3), { steps: calls, nextStep: 6, loading: false }]]);
    const rows = buildRows([turn(1, items)], 1, { view: "all", errorsOnly: false }, new Set(), {
      expanded,
    });
    expect(rows.map((r) => r.type)).toEqual([
      "turn",
      "step",
      "step",
      "batch",
      "member",
      "member",
      "member",
      "more",
      "step",
      "footer",
    ]);
    expect(selectableKeys(rows)).toEqual(["1.1", "1.2", "1.3", "1.4", "1.5", "1.10"]);
    expect(rowIndexOfStep(rows, 1, 4)).toBe(5);
    // The error filter keeps only a batch's failed calls.
    const errors = buildRows([turn(1, items)], 1, { view: "all", errorsOnly: true }, new Set(), {
      expanded: new Map([[batchKey(1, 3), { steps: calls, loading: false }]]),
    });
    expect(errors.filter((r) => r.type === "member")).toHaveLength(0);
  });

  it("shows a sub-agent's steps one level deep, outside the j/k walk", () => {
    const agent = turn(1, [step(1, "agent", { child_session_id: "session_child" })]);
    const child = [step(1, "model"), step(2, "answer")] as TraceStep[];
    const rows = buildRows([agent], 1, { view: "all", errorsOnly: false }, new Set(), {
      expanded: new Map([
        [
          stepRowKey(1, 1),
          { steps: child, loading: false, childSessionId: "session_child", hiddenTurns: 2 },
        ],
      ]),
    });
    expect(rows.map((r) => r.type)).toEqual(["turn", "step", "member", "member", "more", "footer"]);
    expect(rows[2]).toMatchObject({ childSessionId: "session_child" });
    expect(selectableKeys(rows)).toEqual(["1.1"]);
  });

  it("folds lifecycle events no step shows and places them by sequence", () => {
    const event = (sequence: number, type: string): TraceEventRef => ({
      id: `event_${sequence}`,
      sequence,
      type,
      ts: "2026-10-10T00:00:01Z",
      size_bytes: 10,
    });
    const lifecycle = lifecycleEvents([
      event(1, "turn.started"),
      event(2, "capability.usage"),
      event(3, "llm.generation"),
      event(4, "file.written"),
      event(5, "file.written"),
      event(6, "tool.completed"),
      event(7, "act.completed"),
    ]);
    expect(lifecycle).toEqual([
      { type: "capability.usage", sequence: 2, ts: "2026-10-10T00:00:01Z", count: 1 },
      { type: "file.written", sequence: 4, ts: "2026-10-10T00:00:01Z", count: 2 },
    ]);
    const t = turn(1, [step(3, "model"), step(6, "tool"), step(9, "answer")]);
    const rows = buildRows([t], 1, { view: "all", errorsOnly: false }, new Set(), {
      lifecycle: new Map([[1, lifecycle]]),
    });
    expect(rows.map((r) => (r.type === "step" ? `s${r.step.step}` : r.type))).toEqual([
      "turn",
      "lifecycle",
      "s3",
      "lifecycle",
      "s6",
      "s9",
      "footer",
    ]);
    const quiet = buildRows([t], 1, { view: "all", errorsOnly: true }, new Set(), {
      lifecycle: new Map([[1, lifecycle]]),
    });
    expect(quiet.some((r) => r.type === "lifecycle")).toBe(false);
  });

  it("maps a search hit's sequence to its turn and step", () => {
    const first = { ...turn(4, [step(1, "model"), step(2, "tool", { end_sequence: 14 })]) };
    first.start_sequence = 10;
    first.items = [
      { ...(step(1, "model") as TraceStep), type: "step", start_sequence: 10, end_sequence: 11 },
      { ...(step(2, "tool") as TraceStep), type: "step", start_sequence: 12, end_sequence: 14 },
    ];
    const second = { ...turn(5, []), start_sequence: 20 };
    expect(turnHoldingSequence([first, second], 13)?.turn).toBe(4);
    expect(turnHoldingSequence([first, second], 25)?.turn).toBe(5);
    expect(stepHoldingSequence(first, 13)?.step).toBe(2);
    expect(stepHoldingSequence(first, 19)?.step).toBe(2);
    expect(stepHoldingSequence(first, 11)?.step).toBe(1);
    expect(stepHoldingSequence(first, 5)).toBeUndefined();
  });

  it("cuts a search snippet around the match", () => {
    const data = { text: `${"a ".repeat(100)}needle in the haystack` };
    const snippet = searchSnippet(data, "Needle haystack", 40);
    expect(snippet).toContain("needle");
    expect(snippet.startsWith("…")).toBe(true);
    expect(searchSnippet({ x: 1 }, "missing")).toBe('{"x":1}');
  });
});
