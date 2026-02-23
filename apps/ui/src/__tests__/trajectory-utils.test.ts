import { buildTurns, buildTrajectory, countStructuralEvents } from "@/components/trajectory/trajectory-utils";
import type { Event, EventContext } from "@/lib/api/types";

// --- Helpers ---

let seqCounter = 0;

function resetSeq() {
  seqCounter = 0;
}

function emptyContext(): EventContext {
  return {};
}

function turnContext(turnId: string, inputMessageId: string): EventContext {
  return { turn_id: turnId, input_message_id: inputMessageId };
}

function makeEvent(
  type: string,
  data: Record<string, unknown>,
  context: EventContext = emptyContext(),
): Event {
  seqCounter++;
  return {
    id: `evt-${seqCounter}`,
    type,
    ts: `2025-01-01T00:00:${String(seqCounter).padStart(2, "0")}Z`,
    session_id: "session-1",
    context,
    data,
    sequence: seqCounter,
  };
}

function inputMessageEvent(messageId: string, text: string, context?: EventContext): Event {
  return makeEvent(
    "input.message",
    {
      message: {
        id: messageId,
        role: "user",
        content: [{ type: "text", text }],
      },
    },
    context,
  );
}

function turnStartedEvent(turnId: string, inputMessageId: string): Event {
  return makeEvent(
    "turn.started",
    { turn_id: turnId, input_message_id: inputMessageId },
    turnContext(turnId, inputMessageId),
  );
}

function reasonCompletedEvent(turnId: string, inputMessageId: string): Event {
  return makeEvent(
    "reason.completed",
    { has_tool_calls: true, tool_call_count: 1, duration_ms: 100 },
    turnContext(turnId, inputMessageId),
  );
}

function actStartedEvent(turnId: string, inputMessageId: string): Event {
  return makeEvent(
    "act.started",
    {
      tool_calls: [
        { id: "tc-1", name: "bash", display_name: "Bash" },
      ],
    },
    turnContext(turnId, inputMessageId),
  );
}

function toolCompletedEvent(turnId: string, inputMessageId: string): Event {
  return makeEvent(
    "tool.completed",
    {
      tool_call_id: "tc-1",
      name: "bash",
      display_name: "Bash",
      success: true,
      status: "completed",
      duration_ms: 50,
    },
    turnContext(turnId, inputMessageId),
  );
}

function actCompletedEvent(turnId: string, inputMessageId: string): Event {
  return makeEvent(
    "act.completed",
    { success_count: 1, error_count: 0, duration_ms: 50 },
    turnContext(turnId, inputMessageId),
  );
}

function outputMessageCompletedEvent(turnId: string, inputMessageId: string, text: string): Event {
  return makeEvent(
    "output.message.completed",
    {
      message: {
        id: `msg-out-${seqCounter}`,
        role: "agent",
        content: [{ type: "text", text }],
      },
    },
    turnContext(turnId, inputMessageId),
  );
}

function turnCompletedEvent(
  turnId: string,
  inputMessageId: string,
  iterations: number = 1,
): Event {
  return makeEvent(
    "turn.completed",
    { turn_id: turnId, duration_ms: 500, iterations },
    turnContext(turnId, inputMessageId),
  );
}

function turnFailedEvent(turnId: string, inputMessageId: string, error: string): Event {
  return makeEvent(
    "turn.failed",
    { turn_id: turnId, error },
    turnContext(turnId, inputMessageId),
  );
}

// --- Tests ---

describe("trajectory-utils", () => {
  beforeEach(resetSeq);

  describe("buildTurns", () => {
    it("groups input.message + turn.started into single turn (orphan adoption)", () => {
      const events = [
        inputMessageEvent("msg-1", "Hello"),
        turnStartedEvent("turn-1", "msg-1"),
        reasonCompletedEvent("turn-1", "msg-1"),
        outputMessageCompletedEvent("turn-1", "msg-1", "Hi there"),
        turnCompletedEvent("turn-1", "msg-1"),
      ];

      const turns = buildTurns(events);

      expect(turns).toHaveLength(1);
      expect(turns[0].turnId).toBe("turn-1");
      expect(turns[0].userMessage?.text).toBe("Hello");
      expect(turns[0].agentMessage?.text).toBe("Hi there");
      expect(turns[0].completed).toBe(true);
    });

    it("groups turn.started + input.message in normal order (no orphan)", () => {
      const events = [
        turnStartedEvent("turn-1", "msg-1"),
        inputMessageEvent("msg-1", "Hello"),
        reasonCompletedEvent("turn-1", "msg-1"),
        outputMessageCompletedEvent("turn-1", "msg-1", "Hi there"),
        turnCompletedEvent("turn-1", "msg-1"),
      ];

      const turns = buildTurns(events);

      expect(turns).toHaveLength(1);
      expect(turns[0].turnId).toBe("turn-1");
      expect(turns[0].userMessage?.text).toBe("Hello");
      expect(turns[0].agentMessage?.text).toBe("Hi there");
    });

    it("handles multiple turns with orphan adoption", () => {
      const events = [
        // Turn 1: orphan pattern
        inputMessageEvent("msg-1", "First"),
        turnStartedEvent("turn-1", "msg-1"),
        reasonCompletedEvent("turn-1", "msg-1"),
        outputMessageCompletedEvent("turn-1", "msg-1", "Response 1"),
        turnCompletedEvent("turn-1", "msg-1"),
        // Turn 2: orphan pattern
        inputMessageEvent("msg-2", "Second"),
        turnStartedEvent("turn-2", "msg-2"),
        reasonCompletedEvent("turn-2", "msg-2"),
        outputMessageCompletedEvent("turn-2", "msg-2", "Response 2"),
        turnCompletedEvent("turn-2", "msg-2"),
      ];

      const turns = buildTurns(events);

      expect(turns).toHaveLength(2);
      expect(turns[0].turnId).toBe("turn-1");
      expect(turns[0].userMessage?.text).toBe("First");
      expect(turns[0].agentMessage?.text).toBe("Response 1");
      expect(turns[1].turnId).toBe("turn-2");
      expect(turns[1].userMessage?.text).toBe("Second");
      expect(turns[1].agentMessage?.text).toBe("Response 2");
    });

    it("keeps orphan turn when input_message_id does not match", () => {
      const events = [
        inputMessageEvent("msg-1", "Hello"),
        turnStartedEvent("turn-1", "msg-different"),
        reasonCompletedEvent("turn-1", "msg-different"),
        turnCompletedEvent("turn-1", "msg-different"),
      ];

      const turns = buildTurns(events);

      // Orphan not adopted, so two turns
      expect(turns).toHaveLength(2);
      expect(turns[0].turnId).toMatch(/^orphan-/);
      expect(turns[0].userMessage?.text).toBe("Hello");
      expect(turns[1].turnId).toBe("turn-1");
    });

    it("handles turn with tool calls", () => {
      const events = [
        inputMessageEvent("msg-1", "Run a command"),
        turnStartedEvent("turn-1", "msg-1"),
        reasonCompletedEvent("turn-1", "msg-1"),
        actStartedEvent("turn-1", "msg-1"),
        toolCompletedEvent("turn-1", "msg-1"),
        actCompletedEvent("turn-1", "msg-1"),
        outputMessageCompletedEvent("turn-1", "msg-1", "Done"),
        turnCompletedEvent("turn-1", "msg-1"),
      ];

      const turns = buildTurns(events);

      expect(turns).toHaveLength(1);
      expect(turns[0].turnId).toBe("turn-1");
      expect(turns[0].userMessage?.text).toBe("Run a command");
      expect(turns[0].iterations).toHaveLength(1);
      expect(turns[0].iterations[0].toolGroup?.tools).toHaveLength(1);
      expect(turns[0].iterations[0].toolGroup?.tools[0].name).toBe("bash");
      expect(turns[0].agentMessage?.text).toBe("Done");
    });

    it("handles failed turn", () => {
      const events = [
        inputMessageEvent("msg-1", "Break"),
        turnStartedEvent("turn-1", "msg-1"),
        turnFailedEvent("turn-1", "msg-1", "Something went wrong"),
      ];

      const turns = buildTurns(events);

      expect(turns).toHaveLength(1);
      expect(turns[0].turnId).toBe("turn-1");
      expect(turns[0].failed).toBe(true);
      expect(turns[0].errorMessage).toBe("Something went wrong");
      expect(turns[0].userMessage?.text).toBe("Break");
    });

    it("handles empty events", () => {
      const turns = buildTurns([]);
      expect(turns).toHaveLength(0);
    });

    it("preserves orphan messageId after adoption", () => {
      const events = [
        inputMessageEvent("msg-1", "Hello"),
        turnStartedEvent("turn-1", "msg-1"),
        turnCompletedEvent("turn-1", "msg-1"),
      ];

      const turns = buildTurns(events);

      expect(turns).toHaveLength(1);
      expect(turns[0].userMessage?.messageId).toBe("msg-1");
      expect(turns[0].inputMessageId).toBe("msg-1");
    });

    it("handles input.message with context.turn_id (no orphan needed)", () => {
      // When input.message already has turn context (e.g. from scheduler)
      const ctx = turnContext("turn-1", "msg-1");
      const events = [
        turnStartedEvent("turn-1", "msg-1"),
        inputMessageEvent("msg-1", "Hello", ctx),
        reasonCompletedEvent("turn-1", "msg-1"),
        outputMessageCompletedEvent("turn-1", "msg-1", "Hi"),
        turnCompletedEvent("turn-1", "msg-1"),
      ];

      const turns = buildTurns(events);

      expect(turns).toHaveLength(1);
      expect(turns[0].turnId).toBe("turn-1");
      expect(turns[0].userMessage?.text).toBe("Hello");
      expect(turns[0].agentMessage?.text).toBe("Hi");
    });

    it("handles orphan with no message id (no match possible)", () => {
      const events = [
        // input.message with no message.id
        makeEvent("input.message", {
          message: { role: "user", content: [{ type: "text", text: "Hi" }] },
        }),
        turnStartedEvent("turn-1", "msg-1"),
        turnCompletedEvent("turn-1", "msg-1"),
      ];

      const turns = buildTurns(events);

      // Can't match without messageId, so orphan stays separate
      expect(turns).toHaveLength(2);
      expect(turns[0].turnId).toMatch(/^orphan-/);
      expect(turns[1].turnId).toBe("turn-1");
    });
  });

  describe("buildTrajectory", () => {
    it("creates correct nodes for a single turn with orphan adoption", () => {
      const events = [
        inputMessageEvent("msg-1", "Hello"),
        turnStartedEvent("turn-1", "msg-1"),
        reasonCompletedEvent("turn-1", "msg-1"),
        outputMessageCompletedEvent("turn-1", "msg-1", "Hi there"),
        turnCompletedEvent("turn-1", "msg-1"),
      ];

      const { nodes, stats } = buildTrajectory(events);

      // Should have: 1 group + 1 user + 1 reasoning + 1 agent = 4 nodes
      const groupNodes = nodes.filter((n) => n.type === "turnGroup");
      const userNodes = nodes.filter((n) => n.type === "userMessage");
      const agentNodes = nodes.filter((n) => n.type === "agentMessage");

      expect(groupNodes).toHaveLength(1);
      expect(userNodes).toHaveLength(1);
      expect(agentNodes).toHaveLength(1);

      // User and agent should be in the same group
      expect(userNodes[0].parentId).toBe(groupNodes[0].id);
      expect(agentNodes[0].parentId).toBe(groupNodes[0].id);

      expect(stats.turnCount).toBe(1);
      expect(stats.completedTurns).toBe(1);
    });

    it("creates single group for orphan-adopted turn (not two groups)", () => {
      const events = [
        inputMessageEvent("msg-1", "Hello"),
        turnStartedEvent("turn-1", "msg-1"),
        outputMessageCompletedEvent("turn-1", "msg-1", "Hi"),
        turnCompletedEvent("turn-1", "msg-1"),
      ];

      const { nodes, stats } = buildTrajectory(events);

      const groupNodes = nodes.filter((n) => n.type === "turnGroup");
      expect(groupNodes).toHaveLength(1);
      expect(stats.turnCount).toBe(1);
    });

    it("stats reflect merged turns", () => {
      const events = [
        inputMessageEvent("msg-1", "First"),
        turnStartedEvent("turn-1", "msg-1"),
        reasonCompletedEvent("turn-1", "msg-1"),
        actStartedEvent("turn-1", "msg-1"),
        toolCompletedEvent("turn-1", "msg-1"),
        actCompletedEvent("turn-1", "msg-1"),
        outputMessageCompletedEvent("turn-1", "msg-1", "Done 1"),
        turnCompletedEvent("turn-1", "msg-1", 1),
        inputMessageEvent("msg-2", "Second"),
        turnStartedEvent("turn-2", "msg-2"),
        reasonCompletedEvent("turn-2", "msg-2"),
        outputMessageCompletedEvent("turn-2", "msg-2", "Done 2"),
        turnCompletedEvent("turn-2", "msg-2", 1),
      ];

      const { stats } = buildTrajectory(events);

      expect(stats.turnCount).toBe(2);
      expect(stats.completedTurns).toBe(2);
      expect(stats.totalToolCalls).toBe(1);
    });
  });

  describe("countStructuralEvents", () => {
    it("counts only structural events", () => {
      const events = [
        inputMessageEvent("msg-1", "Hello"),
        turnStartedEvent("turn-1", "msg-1"),
        makeEvent("output.message.delta", { text: "partial" }), // not structural
        outputMessageCompletedEvent("turn-1", "msg-1", "Hi"),
        turnCompletedEvent("turn-1", "msg-1"),
      ];

      expect(countStructuralEvents(events)).toBe(4);
    });

    it("returns 0 for undefined", () => {
      expect(countStructuralEvents(undefined)).toBe(0);
    });

    it("returns 0 for empty array", () => {
      expect(countStructuralEvents([])).toBe(0);
    });
  });
});
