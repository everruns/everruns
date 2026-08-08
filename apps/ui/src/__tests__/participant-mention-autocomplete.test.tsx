import { fireEvent, render, screen } from "@testing-library/react";
import {
  getMentionQuery,
  ParticipantMentionAutocomplete,
} from "@/components/chat/participant-mention-autocomplete";

beforeAll(() => {
  Object.defineProperty(window.HTMLElement.prototype, "scrollIntoView", {
    configurable: true,
    value: jest.fn(),
  });
});

describe("getMentionQuery", () => {
  it("finds a mention at the start or after whitespace", () => {
    expect(getMentionQuery("@rese", 5)).toEqual({ start: 0, end: 5, query: "rese" });
    expect(getMentionQuery("ask @help later", 9)).toEqual({ start: 4, end: 9, query: "help" });
  });

  it("does not treat email addresses or completed mention text as queries", () => {
    expect(getMentionQuery("person@example.com", 18)).toBeNull();
    expect(getMentionQuery("@helper continue", 16)).toBeNull();
  });
});

describe("ParticipantMentionAutocomplete", () => {
  it("supports keyboard navigation and selection", () => {
    const onSelect = jest.fn();
    render(
      <ParticipantMentionAutocomplete
        options={[
          { id: "part_one", label: "Researcher" },
          { id: "part_two", label: "Writer" },
        ]}
        query=""
        visible
        onDismiss={jest.fn()}
        onSelect={onSelect}
      />,
    );

    fireEvent.keyDown(document, { key: "ArrowDown" });
    fireEvent.keyDown(document, { key: "Enter" });

    expect(onSelect).toHaveBeenCalledWith({ id: "part_two", label: "Writer" });
  });

  it("announces a no-match state and dismisses it with Escape", () => {
    const onDismiss = jest.fn();
    render(
      <ParticipantMentionAutocomplete
        options={[{ id: "part_one", label: "Researcher" }]}
        query="writer"
        visible
        onDismiss={onDismiss}
        onSelect={jest.fn()}
      />,
    );

    expect(screen.getByText("No matching participants")).toBeInTheDocument();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });
});
