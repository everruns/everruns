import { render, screen } from "@testing-library/react";
import { SessionRow } from "@/components/session/session-row";
import type { Session } from "@/lib/api/types";

const LONG_ERROR =
  "The AI provider account is out of credits for this request: it asked for up to 64000 output tokens, but the balance covers none.";

function failedSession(): Session {
  return {
    id: "session_01933b5a00007000800000000000001",
    organization_id: "org_1",
    harness_id: "harness_1",
    agent_id: "agent_1",
    owner_principal_id: "principal_1",
    title: "Cloudflare setup",
    output_preview: LONG_ERROR,
    tags: [],
    model_id: null,
    status: "idle",
    activity: "failed",
    created_at: "2026-11-18T12:00:00Z",
    updated_at: "2026-11-18T12:01:00Z",
    started_at: "2026-11-18T12:00:00Z",
    finished_at: "2026-11-18T12:01:00Z",
  };
}

describe("SessionRow", () => {
  it("constrains a long failure message to its outcome column", () => {
    render(<SessionRow session={failedSession()} agentName="Worker web agent" />);

    const error = screen.getByText(LONG_ERROR);
    expect(error).toHaveClass("block", "truncate");
    expect(error).toHaveAttribute("title", LONG_ERROR);
  });
});
