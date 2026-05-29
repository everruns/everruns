import { fireEvent, render, screen } from "@testing-library/react";
import { TurnWorkLog } from "@/components/chat/turn-work-log";

describe("TurnWorkLog", () => {
  it("starts collapsed for completed work", () => {
    render(
      <TurnWorkLog label="Worked for 2m 4s" isActive={false}>
        <div>Read files and ran tests</div>
      </TurnWorkLog>,
    );

    const button = screen.getByRole("button", { name: /worked for 2m 4s/i });
    expect(button).toHaveAttribute("aria-expanded", "false");
  });

  it("stays open while active and collapses after completion", () => {
    const { rerender } = render(
      <TurnWorkLog label="Working" isActive>
        <div>Inspecting the repo</div>
      </TurnWorkLog>,
    );

    expect(screen.getByRole("button", { name: /working/i })).toHaveAttribute(
      "aria-expanded",
      "true",
    );

    rerender(
      <TurnWorkLog label="Worked for 18s" isActive={false}>
        <div>Inspecting the repo</div>
      </TurnWorkLog>,
    );

    expect(screen.getByRole("button", { name: /worked for 18s/i })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
  });

  it("can be reopened after auto-collapse", () => {
    render(
      <TurnWorkLog label="Worked for 18s" isActive={false}>
        <div>Inspecting the repo</div>
      </TurnWorkLog>,
    );

    const button = screen.getByRole("button", { name: /worked for 18s/i });
    fireEvent.click(button);

    expect(button).toHaveAttribute("aria-expanded", "true");
  });
});
