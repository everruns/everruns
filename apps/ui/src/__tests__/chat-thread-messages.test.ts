import { parseCoordinatorMessage, parseTaskUpdate } from "@/lib/chat-thread-messages";

describe("task updates", () => {
  it("drops the task id and field labels from a finished task", () => {
    expect(
      parseTaskUpdate(
        'Task "Three latest releases" (task_01a11d2c) finished: succeeded.\n- summary: Found v0.44.0.\n\nValidation: checked the listing.\n- result_path: /tasks/out.md',
      ),
    ).toEqual({
      kind: "done",
      title: "Three latest releases",
      taskId: "task_01a11d2c",
      body: "Found v0.44.0.\n\nValidation: checked the listing.",
    });
    expect(parseTaskUpdate('Task "Draft" (task_1) finished: failed.')).toEqual({
      kind: "failed",
      title: "Draft",
      taskId: "task_1",
      body: "",
    });
  });
  it("reads questions and messages", () => {
    expect(parseTaskUpdate('Task "Draft" (task_1) is awaiting input: Which tone?')).toEqual({
      kind: "needs_input",
      title: "Draft",
      taskId: "task_1",
      body: "Which tone?",
    });
    expect(parseTaskUpdate('Task "Draft" (task_1) sent a message: halfway')).toEqual({
      kind: "message",
      title: "Draft",
      taskId: "task_1",
      body: "halfway",
    });
  });
  it("leaves anything else as written", () => {
    expect(parseTaskUpdate("Also add the dates.")).toBeNull();
    expect(parseTaskUpdate('Task "Draft" (task_1) finished: exploded.')).toBeNull();
  });
});

describe("coordinator messages", () => {
  it("shows a brief without the worker's instructions", () => {
    expect(
      parseCoordinatorMessage(
        "New assignment from the coordinator: Add release dates\n\nAdd the dates.\n\nKeep your checklist current with update_checklist. When the work is done, call complete_assignment.",
      ),
    ).toEqual({
      kind: "assignment",
      title: "Add release dates",
      body: "Add the dates.",
    });
  });
  it("reads the Threads framing from a parent", () => {
    expect(
      parseCoordinatorMessage(
        "New assignment from your parent: Add release dates\n\nAdd the dates.\n\nKeep your checklist current with report_progress. When the work is done, call finish.",
      ),
    ).toEqual({ kind: "assignment", title: "Add release dates", body: "Add the dates." });
    expect(parseCoordinatorMessage("Message from your parent:\n\nUse B.")).toEqual({
      kind: "relay",
      body: "Use B.",
    });
  });
  it("shows a relayed follow-up as written", () => {
    expect(parseCoordinatorMessage("Message from the coordinator:\n\nUse A.")).toEqual({
      kind: "relay",
      body: "Use A.",
    });
    expect(parseCoordinatorMessage("Hello")).toBeNull();
  });
});
