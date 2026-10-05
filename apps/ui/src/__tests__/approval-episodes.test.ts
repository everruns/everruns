import { buildApprovalEpisodes } from "@/lib/approval-episodes";
import type { AuditLogEntry } from "@/lib/api/audit-logs";
import type { Event } from "@/lib/api/types";

function toolEvent(
  id: string,
  toolName: string,
  payload: Record<string, unknown>,
  sequence: number,
  success = true,
): Event {
  return {
    id,
    type: "tool.completed",
    ts: `2026-10-0${sequence}T22:56:${String(20 + sequence).padStart(2, "0")}Z`,
    sequence,
    session_id: "session_1",
    context: {},
    data: {
      tool_call_id: id,
      tool_name: toolName,
      success,
      status: success ? "success" : "error",
      result: success ? [{ type: "text", text: JSON.stringify(payload) }] : undefined,
      error: success ? undefined : "failed",
    },
  };
}

function inputEvent(id: string, sequence: number, userId?: string): Event {
  return {
    id: `event_${id}`,
    type: "input.message",
    ts: `2026-10-0${sequence}T22:56:${String(10 + sequence).padStart(2, "0")}Z`,
    sequence,
    session_id: "session_1",
    context: {},
    metadata: userId
      ? { initiator: { type: "user", user_id: userId } }
      : { initiator: { type: "schedule" } },
    data: {
      message: {
        id,
        session_id: "session_1",
        sequence,
        role: "user",
        content: [{ type: "text", text: "yes" }],
        tool_call_id: null,
        created_at: `2026-10-0${sequence}T22:56:${String(10 + sequence).padStart(2, "0")}Z`,
      },
    },
  };
}

const ASK =
  'Create the reusable organisation-wide agent "SRE Simulator" using the built-in Generic harness and the proposed simulation-only instructions';
const QUESTION =
  "Create the SRE Simulator agent with the proposed safe, simulation-only configuration?";
const GRANT =
  "Create the organisation-wide SRE Simulator agent using the built-in Generic harness and simulation-only instructions";
const DETAIL =
  "Approved configuration: safety-first, simulation-only SRE training and analysis agent with no external integrations.";

describe("buildApprovalEpisodes", () => {
  it("pairs a request with the later grant even when the wording differs", () => {
    const episodes = buildApprovalEpisodes(
      [
        toolEvent("ask", "request_approval", { action: ASK, question: QUESTION }, 1),
        inputEvent("msg_consent", 2, "user_mykhailo"),
        toolEvent(
          "grant",
          "record_approval",
          { action: GRANT, detail: DETAIL, approved_in_message: "msg_consent" },
          3,
        ),
      ],
      { memberNames: new Map([["user_mykhailo", "Mykhailo Chalyi"]]), inputsComplete: true },
    );

    expect(episodes).toHaveLength(1);
    expect(episodes[0]).toMatchObject({
      id: "grant",
      status: "approved",
      recordedWithoutRequest: false,
      awaitingConsent: false,
      ask: { action: ASK, question: QUESTION },
      grant: {
        action: GRANT,
        detail: DETAIL,
        approvedBy: "Mykhailo Chalyi",
        consentMessageId: "msg_consent",
      },
    });
  });

  it("gives a grant to the oldest open request", () => {
    const episodes = buildApprovalEpisodes(
      [
        toolEvent("ask-1", "request_approval", { action: "drop staging" }, 1),
        toolEvent("ask-2", "request_approval", { action: "drop prod" }, 2),
        toolEvent("grant", "record_approval", { action: "dropped staging" }, 3),
      ],
      { inputsComplete: true },
    );

    expect(episodes.map((episode) => episode.status)).toEqual(["approved", "open"]);
    expect(episodes.find((episode) => episode.status === "approved")?.ask?.action).toBe(
      "drop staging",
    );
    expect(episodes.find((episode) => episode.status === "open")?.ask?.action).toBe("drop prod");
  });

  it("keeps a grant that has no preceding request", () => {
    const episodes = buildApprovalEpisodes(
      [
        toolEvent("grant", "record_approval", { action: "commits need no further ask" }, 1),
        toolEvent("ask", "request_approval", { action: "delete the bucket" }, 2),
      ],
      { inputsComplete: true },
    );

    const recorded = episodes.find((episode) => episode.grant);
    const open = episodes.find((episode) => episode.status === "open");
    expect(recorded).toMatchObject({ recordedWithoutRequest: true, ask: undefined });
    expect(open?.ask?.action).toBe("delete the bucket");
    expect(open?.awaitingConsent).toBe(true);
  });

  it("does not call an answered request still waiting", () => {
    const episodes = buildApprovalEpisodes(
      [
        toolEvent("ask", "request_approval", { action: "ship it", question: "Ship?" }, 1),
        inputEvent("msg_no", 2, "user_mykhailo"),
      ],
      { inputsComplete: true },
    );

    expect(episodes[0]).toMatchObject({ status: "open", awaitingConsent: false });
  });

  it("does not guess that an ask outside the loaded window is still waiting", () => {
    const episodes = buildApprovalEpisodes(
      [toolEvent("ask", "request_approval", { action: "ship it" }, 1)],
      { inputsComplete: false, loadedSinceSequence: 50 },
    );

    expect(episodes[0]?.awaitingConsent).toBe(false);
  });

  it("names the approver from the audit row when the consent message is not loaded", () => {
    const audit: AuditLogEntry = {
      id: "audit_1",
      domain: "agent",
      action: "agent.approval.granted",
      actor_id: "user_mykhailo",
      event_type: "agent.approval.granted",
      target_type: "session",
      target_id: "session_1",
      ip_address: null,
      metadata: { approved_in_message: "msg_consent" },
      created_at: "2026-10-03T22:56:43Z",
    };
    const episodes = buildApprovalEpisodes(
      [
        toolEvent(
          "grant",
          "record_approval",
          { action: GRANT, approved_in_message: "msg_consent" },
          2,
        ),
      ],
      { memberNames: new Map([["user_mykhailo", "Mykhailo Chalyi"]]), auditEntries: [audit] },
    );

    expect(episodes[0]?.grant?.approvedBy).toBe("Mykhailo Chalyi");
  });

  it("ignores failed approval calls and results that are not the tool payload", () => {
    const episodes = buildApprovalEpisodes([
      toolEvent("failed", "request_approval", { action: "nope" }, 1, false),
      {
        ...toolEvent("bad-json", "record_approval", { action: "unused" }, 3),
        data: {
          tool_call_id: "bad-json",
          tool_name: "record_approval",
          success: true,
          status: "success",
          result: [{ type: "text", text: "not json" }],
        },
      },
    ]);

    expect(episodes).toEqual([]);
  });

  it("keeps a grant that named no action", () => {
    const episodes = buildApprovalEpisodes([toolEvent("grant", "record_approval", {}, 1)]);

    expect(episodes[0]?.grant?.action).toBe("Critical action");
    expect(episodes[0]?.recordedWithoutRequest).toBe(true);
  });
});
