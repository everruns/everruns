// The Timeline tab's structural rail (EVE-854): the run as it happened, derived
// purely from the event stream plus the session's task records, so a finished
// session renders fully from history with no live connection.

import { fireEvent, render, screen, within } from "@testing-library/react";
import React from "react";
import { RunTimeline } from "@/components/session/run-timeline";
import type { Event, SessionTask } from "@/lib/api/types";

function event(sequence: number, type: string, data: Record<string, unknown>): Event {
  return {
    id: `evt_${sequence}`,
    type,
    ts: new Date(Date.UTC(2026, 4, 25, 10, 0, sequence)).toISOString(),
    session_id: "session_1",
    context: { exec_id: "exec_alpha" },
    sequence,
    data,
  } as Event;
}

const events: Event[] = [
  event(1, "session.started", { agent_id: "agent_1", model_id: "model_1" }),
  event(2, "turn.started", { turn_id: "turn_1", input_message_id: "msg_1" }),
  event(3, "llm.generation", {
    messages: [],
    output: { tool_calls: [] },
    metadata: {
      model: "gpt-4",
      success: true,
      usage: { input_tokens: 1200, output_tokens: 80 },
      retry: { attempts: 2, total_wait_ms: 500 },
    },
  }),
  event(4, "tool.completed", {
    tool_call_id: "call_1",
    tool_name: "read_file",
    display_name: "Read file",
    success: true,
    status: "success",
    result: [{ type: "text", text: "the file body" }],
  }),
  event(5, "turn.failed", { turn_id: "turn_1", error: "provider timed out" }),
  event(6, "turn.completed", { turn_id: "turn_1", iterations: 3 }),
  event(7, "session.idled", {
    turn_id: "turn_1",
    usage: { input_tokens: 1200, output_tokens: 80 },
  }),
];

const tasks: SessionTask[] = [
  {
    id: "task_1",
    session_id: "session_1",
    kind: "subagent",
    display_name: "Doc reviewer",
    state: "succeeded",
    summary: "Found two typos",
    worker_id: "worker_7",
    attempt: 1,
    wake_policy: "always",
    created_at: new Date(Date.UTC(2026, 4, 25, 10, 0, 4)).toISOString(),
    updated_at: new Date(Date.UTC(2026, 4, 25, 10, 0, 5)).toISOString(),
  } as unknown as SessionTask,
];

describe("RunTimeline", () => {
  it("renders the run's structural steps in order", () => {
    render(<RunTimeline events={events} tasks={[]} />);

    const titles = screen.getAllByRole("listitem").map((item) => item.textContent ?? "");
    expect(titles[0]).toContain("Session started");
    expect(titles[1]).toContain("Agent turn started");
    expect(titles[2]).toContain("Model call");
    expect(titles.at(-1)).toContain("Session closed the turn");
  });

  it("names the model and prompt tokens on each model call", () => {
    render(<RunTimeline events={events} tasks={[]} />);

    expect(screen.getByText("gpt-4")).toBeInTheDocument();
    expect(screen.getByText("1.2K prompt")).toBeInTheDocument();
  });

  it("keeps durable-execution worker IDs behind details", () => {
    render(<RunTimeline events={events} tasks={[]} />);

    expect(screen.queryByText("Worker: exec_alpha")).not.toBeInTheDocument();
    const modelStep = screen.getByText("Model call").closest("li")!;
    fireEvent.click(within(modelStep).getByRole("button", { name: /show details/i }));
    expect(screen.getByText("Worker: exec_alpha")).toBeInTheDocument();
  });

  it("surfaces model retries as a curated fact", () => {
    render(<RunTimeline events={events} tasks={[]} />);

    expect(screen.getByText("3 attempts")).toBeInTheDocument();
  });

  it("shows a tool call's result payload inline, behind an expander", () => {
    render(<RunTimeline events={events} tasks={[]} />);

    expect(screen.getByText("Read file")).toBeInTheDocument();
    expect(screen.queryByText("the file body")).not.toBeInTheDocument();

    const toolStep = screen.getByText("Read file").closest("li")!;
    fireEvent.click(within(toolStep).getByRole("button", { name: /show details/i }));
    expect(screen.getByText("the file body")).toBeInTheDocument();
  });

  it("surfaces failed turns as their own step", () => {
    render(<RunTimeline events={events} tasks={[]} />);

    expect(screen.getByText("Turn failed")).toBeInTheDocument();
  });

  it("interleaves subagents with what they returned", () => {
    render(<RunTimeline events={events} tasks={tasks} />);

    expect(screen.getByText("Doc reviewer")).toBeInTheDocument();
    expect(screen.queryByText("Worker: worker_7")).not.toBeInTheDocument();
    const subagentStep = screen.getByText("Doc reviewer").closest("li")!;
    fireEvent.click(within(subagentStep).getByRole("button", { name: /show details/i }));
    expect(screen.getByText("Worker: worker_7")).toBeInTheDocument();

    const titles = screen.getAllByRole("listitem").map((item) => item.textContent ?? "");
    const subagentIndex = titles.findIndex((title) => title.includes("Doc reviewer"));
    const turnCompletedIndex = titles.findIndex((title) => title.includes("Agent turn completed"));
    expect(subagentIndex).toBeGreaterThan(0);
    expect(subagentIndex).toBeLessThan(turnCompletedIndex);
  });

  it("says so when nothing has run", () => {
    render(<RunTimeline events={[]} tasks={[]} />);

    expect(screen.getByText(/Nothing has run in this session yet/)).toBeInTheDocument();
  });

  it("shows a loading state while live history is being hydrated", () => {
    render(<RunTimeline events={[]} tasks={[]} loading />);

    expect(screen.getByText("Loading the run…")).toBeInTheDocument();
  });
});
