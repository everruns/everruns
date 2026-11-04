import { messageOrigin, readOrigin } from "@/lib/message-origin";

describe("messageOrigin", () => {
  it("reads the stored origin object", () => {
    expect(
      messageOrigin({
        everruns_origin: {
          kind: "agent_trigger",
          source_id: "trg_1",
          name: "GitHub",
          run: { number: 2 },
        },
      }),
    ).toEqual({ kind: "agent_trigger", source_id: "trg_1", name: "GitHub", run: { number: 2 } });
  });

  it("maps the earlier markers to the same shape", () => {
    expect(messageOrigin({ everruns_origin: "task_wake" })).toEqual({ kind: "task_wake" });
    expect(
      messageOrigin({
        source: "schedule",
        schedule_id: "sched_1",
        schedule_run: { number: 3, limit: 96, budget: 2 },
      }),
    ).toEqual({ kind: "schedule", source_id: "sched_1", run: { number: 3, limit: 96 } });
    expect(messageOrigin({ source: "app_webhook", app_channel_id: "ch_1" })?.kind).toBe("webhook");
  });

  it("treats messages a person sent as having no origin", () => {
    for (const metadata of [
      undefined,
      {},
      { source: "voice" },
      { everruns_origin: "something_else" },
      { everruns_origin: { kind: "made_up" } },
    ]) {
      expect(messageOrigin(metadata)).toBeNull();
    }
    expect(readOrigin(null)).toBeNull();
  });
});
