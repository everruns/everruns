import { act, fireEvent, render, screen } from "@testing-library/react";
import { TurnWorkLog } from "@/components/chat/turn-work-log";

describe("TurnWorkLog", () => {
  afterEach(() => {
    jest.useRealTimers();
  });

  it("starts collapsed for completed work", () => {
    render(
      <TurnWorkLog label="Worked for 2m 4s" isActive={false}>
        <div>Read files and ran tests</div>
      </TurnWorkLog>,
    );

    const button = screen.getByRole("button", { name: /worked for 2m 4s/i });
    expect(button).toHaveAttribute("aria-expanded", "false");
  });

  it("stays collapsed while active and shows the latest step instead", () => {
    render(
      <TurnWorkLog label="Working" isActive status="Reading chat-message-list.tsx">
        <div>Inspecting the repo</div>
      </TurnWorkLog>,
    );

    expect(screen.getByRole("button", { name: /working/i })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(screen.getByTestId("work-log-status")).toHaveTextContent(
      "Reading chat-message-list.tsx",
    );
  });

  it("drops the live status once the turn completes", () => {
    const { rerender } = render(
      <TurnWorkLog label="Working" isActive status="Running tests">
        <div>Inspecting the repo</div>
      </TurnWorkLog>,
    );

    rerender(
      <TurnWorkLog label="Worked for 18s" isActive={false} status="Running tests">
        <div>Inspecting the repo</div>
      </TurnWorkLog>,
    );

    expect(screen.queryByTestId("work-log-status")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /worked for 18s/i })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
  });

  it("keeps the user's choice to open it across completion", () => {
    const { rerender } = render(
      <TurnWorkLog label="Working" isActive>
        <div>Inspecting the repo</div>
      </TurnWorkLog>,
    );

    fireEvent.click(screen.getByRole("button", { name: /working/i }));

    rerender(
      <TurnWorkLog label="Worked for 18s" isActive={false}>
        <div>Inspecting the repo</div>
      </TurnWorkLog>,
    );

    expect(screen.getByRole("button", { name: /worked for 18s/i })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
  });

  it("can be reopened after completion", () => {
    render(
      <TurnWorkLog label="Worked for 18s" isActive={false}>
        <div>Inspecting the repo</div>
      </TurnWorkLog>,
    );

    const button = screen.getByRole("button", { name: /worked for 18s/i });
    fireEvent.click(button);

    expect(button).toHaveAttribute("aria-expanded", "true");
  });

  it("surfaces failed tool calls in the collapsed header", () => {
    render(
      <TurnWorkLog label="Worked for 33s" isActive={false} errorCount={2}>
        <div>Creating the support agent</div>
      </TurnWorkLog>,
    );

    expect(screen.getByRole("button", { name: /worked for 33s/i })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(screen.getByTestId("work-log-error-count")).toHaveTextContent("2 errors");
  });

  it("keeps attention content visible outside the fold", () => {
    render(
      <TurnWorkLog label="Working" isActive attention={<div>Approve this tool call</div>}>
        <div>Hidden detail</div>
      </TurnWorkLog>,
    );

    const attention = screen.getByText("Approve this tool call");
    expect(attention.closest("[aria-hidden='true']")).toBeNull();
    expect(screen.getByText("Hidden detail").closest("[aria-hidden='true']")).not.toBeNull();
  });

  it("counts elapsed time while the turn runs", () => {
    jest.useFakeTimers();
    jest.setSystemTime(new Date("2026-10-03T00:00:10Z"));

    render(
      <TurnWorkLog label="Working" isActive startedAtMs={Date.parse("2026-10-03T00:00:00Z")}>
        <div>Inspecting the repo</div>
      </TurnWorkLog>,
    );

    expect(screen.getByRole("button", { name: /working for 10s/i })).toBeInTheDocument();

    act(() => {
      jest.advanceTimersByTime(5000);
    });

    expect(screen.getByRole("button", { name: /working for 15s/i })).toBeInTheDocument();
  });
});
