import { render, screen } from "@testing-library/react";
import { SessionCard } from "@/components/session/session-card";
import type { Session } from "@/lib/api/types";

const session: Session = {
  id: "session_test",
  organization_id: "org_test",
  harness_id: "harness_test",
  agent_id: "agent_test",
  owner_principal_id: "principal_test",
  title: "Support conversation",
  tags: [],
  model_id: null,
  status: "idle",
  source: "slack",
  created_at: "2026-10-12T10:00:00Z",
  updated_at: "2026-10-16T10:00:00Z",
  started_at: null,
  finished_at: null,
};

it("shows channel and distinct creation and last-message timestamps", () => {
  const lastMessageAt = "2026-10-15T15:30:00Z";
  render(<SessionCard session={{ ...session, last_message_at: lastMessageAt }} />);

  expect(screen.getByText("Channel: Slack")).toBeInTheDocument();
  expect(screen.getByText(/^Created /).closest("time")).toHaveAttribute(
    "dateTime",
    session.created_at,
  );
  expect(screen.getByText(/^Last message /).closest("time")).toHaveAttribute(
    "dateTime",
    lastMessageAt,
  );
});

it("does not present an unrelated session update as a message", () => {
  render(<SessionCard session={{ ...session, source: undefined }} />);

  expect(screen.getByText("Channel: Unknown")).toBeInTheDocument();
  expect(screen.getByText("No messages yet")).toBeInTheDocument();
  expect(screen.queryByText(/^Last message /)).not.toBeInTheDocument();
});

it("marks archived sessions while keeping their execution status", () => {
  render(<SessionCard session={{ ...session, archived_at: session.updated_at }} />);

  expect(screen.getByText("Archived")).toBeInTheDocument();
  expect(screen.getByText("Idle")).toBeInTheDocument();
});

it("retains failure details beside the channel and last-message time", () => {
  render(
    <SessionCard
      session={{
        ...session,
        activity: "failed",
        last_turn_status: "failed",
        last_message_at: session.created_at,
      }}
      failure={{ count: 1, at: session.updated_at, error: "Model request timed out", sequence: 42 }}
    />,
  );

  expect(screen.getByText("Last turn failed")).toBeInTheDocument();
  expect(screen.getByText("Model request timed out")).toBeInTheDocument();
  expect(screen.getByRole("link", { name: "View failure" })).toHaveAttribute(
    "href",
    "/sessions/session_test/trace?failure=42",
  );
  expect(screen.getByText("Channel: Slack")).toBeInTheDocument();
  expect(screen.getByText(/^Last message /).closest("time")).toHaveAttribute(
    "dateTime",
    session.created_at,
  );
});
