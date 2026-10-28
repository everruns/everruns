"use client";
export type ChatEvent = {
  id?: string;
  sequence?: number;
  ts?: string;
  type: string;
  data: Record<string, unknown>;
};
export function workForMessages(events: ChatEvent[]) {
  const logs = new Map<string, ChatEvent[]>();
  let current: ChatEvent[] = [];
  for (const event of events) {
    if (event.type === "input.message") current = [];
    if (event.type === "tool.started" || event.type === "tool.completed") current.push(event);
    if (event.type.startsWith("output.message.")) {
      const message = event.data.message as { id?: string } | undefined;
      const id = message?.id ?? event.data.message_id ?? event.data.id;
      if (typeof id === "string") logs.set(id, [...current]);
    }
  }
  return { logs, pending: current };
}
export function WorkLog({ events, running = false }: { events: ChatEvent[]; running?: boolean }) {
  const steps = new Map<string, { label: string; done: boolean; failed: boolean }>();
  for (const event of events) {
    const call = event.data.tool_call as { id?: string; name?: string } | undefined;
    const id = String(call?.id ?? event.data.tool_call_id ?? event.id);
    const old = steps.get(id);
    steps.set(id, {
      label:
        old?.label ??
        String(
          event.data.narration ??
            event.data.display_name ??
            event.data.tool_name ??
            call?.name ??
            event.data.activity ??
            "Tool activity",
        ),
      done: event.type === "tool.completed",
      failed: event.data.success === false,
    });
  }
  if (!steps.size && !running) return null;
  return (
    <details className="activity">
      <summary>
        {running ? "Working" : "Worked"} · {steps.size} {steps.size === 1 ? "step" : "steps"}
      </summary>
      {[...steps.entries()].map(([id, step]) => (
        <div key={id}>
          <span aria-label={step.failed ? "Failed" : step.done ? "Completed" : "Running"}>
            {step.failed ? "!" : step.done ? "✓" : "◌"}
          </span>{" "}
          {step.label}
        </div>
      ))}
    </details>
  );
}
