import { assignRunsToEvents, runDurationMs, type ChatRun } from "@/components/chat/run-cards";
import type { Event, SessionTask } from "@/lib/api/types";

let sequence = 0;

function event(type: string, ts: string, data: Record<string, unknown>, turnId?: string): Event {
  sequence += 1;
  return {
    id: `evt_${sequence}`,
    type,
    ts,
    session_id: "sess_1",
    context: turnId ? { turn_id: turnId } : {},
    data,
  } as Event;
}

/** One complete turn: input row, turn.started, an answer row, turn.completed. */
function turn(turnId: string, startTs: string, answerTs: string, endTs: string) {
  const input = event(
    "input.message",
    startTs,
    { message: { id: `msg_${turnId}`, content: [{ type: "text", text: "hi" }] } },
    turnId,
  );
  const started = event(
    "turn.started",
    startTs,
    { turn_id: turnId, input_message_id: `msg_${turnId}` },
    turnId,
  );
  const answer = event(
    "output.message.completed",
    answerTs,
    { message: { id: `out_${turnId}`, content: [{ type: "text", text: "done" }] } },
    turnId,
  );
  const completed = event(
    "turn.completed",
    endTs,
    { turn_id: turnId, iterations: 1, duration_ms: 1000 },
    turnId,
  );
  return { events: [input, started, answer, completed], answerId: answer.id };
}

function run(id: string, createdAt: string, overrides: Partial<SessionTask> = {}): ChatRun {
  return {
    task: {
      id,
      session_id: "sess_1",
      kind: "subagent",
      display_name: id,
      spec: {},
      state: "succeeded",
      attempt: 1,
      wake_policy: "on_terminal",
      created_at: createdAt,
      updated_at: createdAt,
      ...overrides,
    } as SessionTask,
    subagentCount: 0,
  };
}

describe("assignRunsToEvents", () => {
  const first = turn(
    "turn_1",
    "2026-08-09T10:00:00Z",
    "2026-08-09T10:00:20Z",
    "2026-08-09T10:00:30Z",
  );
  const second = turn(
    "turn_2",
    "2026-08-09T11:00:00Z",
    "2026-08-09T11:00:20Z",
    "2026-08-09T11:00:30Z",
  );
  const events = [...first.events, ...second.events];

  it("hangs a run off the last row of the turn that started it", () => {
    const map = assignRunsToEvents(events, [run("task_1", "2026-08-09T10:00:10Z")]);

    expect([...map.keys()]).toEqual([first.answerId]);
    expect(map.get(first.answerId)?.[0].task.id).toBe("task_1");
  });

  it("keeps runs with the turn they belong to", () => {
    const map = assignRunsToEvents(events, [
      run("task_1", "2026-08-09T10:00:10Z"),
      run("task_2", "2026-08-09T11:00:05Z"),
      run("task_3", "2026-08-09T11:00:10Z"),
    ]);

    expect(map.get(first.answerId)?.map((r) => r.task.id)).toEqual(["task_1"]);
    expect(map.get(second.answerId)?.map((r) => r.task.id)).toEqual(["task_2", "task_3"]);
  });

  it("places a run started by the turn still in flight", () => {
    const running = turn("turn_3", "2026-08-09T12:00:00Z", "2026-08-09T12:00:05Z", "");
    const openEvents = [...running.events.slice(0, 3)]; // no turn.completed yet
    const map = assignRunsToEvents(openEvents, [run("task_4", "2026-08-09T12:30:00Z")]);

    expect(map.get(running.answerId)?.map((r) => r.task.id)).toEqual(["task_4"]);
  });

  it("leaves a task created outside any turn unplaced rather than guessing", () => {
    const map = assignRunsToEvents(events, [
      run("before", "2026-08-09T09:00:00Z"),
      run("between", "2026-08-09T10:45:00Z"),
    ]);

    expect(map.size).toBe(0);
  });

  it("does nothing without runs or without turns", () => {
    expect(assignRunsToEvents(events, []).size).toBe(0);
    expect(assignRunsToEvents([], [run("task_1", "2026-08-09T10:00:10Z")]).size).toBe(0);
  });
});

describe("runDurationMs", () => {
  const now = Date.parse("2026-08-09T10:00:30Z");

  it("is null before the run starts", () => {
    expect(runDurationMs(run("t", "2026-08-09T10:00:00Z").task, now)).toBeNull();
  });

  it("measures a finished run end to end", () => {
    const task = run("t", "2026-08-09T10:00:00Z", {
      started_at: "2026-08-09T10:00:00Z",
      finished_at: "2026-08-09T10:00:12Z",
    }).task;

    expect(runDurationMs(task, now)).toBe(12_000);
  });

  it("measures a running run against the clock", () => {
    const task = run("t", "2026-08-09T10:00:00Z", { started_at: "2026-08-09T10:00:00Z" }).task;

    expect(runDurationMs(task, now)).toBe(30_000);
  });
});
