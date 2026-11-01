import { fireEvent, render, screen } from "@testing-library/react";
import { ReactionChips } from "@/components/chat/message-reactions";
import { applyReaction, type MessageReactions } from "@/lib/api/message-reactions";

jest.mock("@/providers/locale-provider", () => ({
  ...jest.requireActual("@/providers/locale-provider"),
  useLocale: () => ({
    locale: "en",
    t: (key: string, values?: Record<string, string>) =>
      values ? `${key}:${Object.values(values).join("|")}` : key,
  }),
}));

const actor = (name: string) => ({
  kind: "user" as const,
  id: name,
  name,
  reacted_at: "2026-11-01T00:00:00Z",
});

const reactions: MessageReactions[] = [
  {
    message_id: "msg_1",
    reactions: [
      {
        emoji: "👍",
        count: 2,
        me: true,
        agent: false,
        actors: [actor("Mykhailo"), actor("Alice")],
      },
      { emoji: "🎉", count: 1, me: false, agent: false, actors: [actor("Alice")] },
    ],
  },
];

describe("applyReaction", () => {
  it("adds the caller to an existing emoji and creates a new one", () => {
    const next = applyReaction(reactions, "msg_1", "🎉", true);
    expect(next[0].reactions.find((r) => r.emoji === "🎉")).toMatchObject({ count: 2, me: true });
    const fresh = applyReaction([], "msg_2", "👀", true);
    expect(fresh).toEqual([
      {
        message_id: "msg_2",
        reactions: [{ emoji: "👀", count: 1, me: true, agent: false, actors: [] }],
      },
    ]);
  });

  it("removes the caller and drops an emoji nobody has left", () => {
    const next = applyReaction(reactions, "msg_1", "👍", false);
    expect(next[0].reactions.find((r) => r.emoji === "👍")).toMatchObject({ count: 1, me: false });
    const only = applyReaction(applyReaction([], "msg_2", "👀", true), "msg_2", "👀", false);
    expect(only).toEqual([]);
  });

  it("changes nothing when the caller's state already matches", () => {
    expect(applyReaction(reactions, "msg_1", "👍", true)).toBe(reactions);
    expect(applyReaction(reactions, "msg_1", "🎉", false)).toBe(reactions);
  });
});

describe("ReactionChips", () => {
  it("marks the caller's chip and toggles it", () => {
    const onToggle = jest.fn();
    render(<ReactionChips reactions={reactions[0].reactions} onToggle={onToggle} />);
    const [thumbs, party] = screen.getAllByTestId("reaction-chip");
    expect(thumbs).toHaveAttribute("aria-pressed", "true");
    expect(thumbs).toHaveTextContent("2");
    expect(party).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(thumbs);
    expect(onToggle).toHaveBeenCalledWith("👍", false);
    fireEvent.click(party);
    expect(onToggle).toHaveBeenCalledWith("🎉", true);
    expect(screen.getByTestId("reaction-chip-add")).toBeInTheDocument();
  });

  it("is read-only without a toggle and names who reacted", () => {
    render(<ReactionChips reactions={reactions[0].reactions} />);
    const [thumbs] = screen.getAllByTestId("reaction-chip");
    expect(thumbs).toBeDisabled();
    expect(thumbs).toHaveAttribute("aria-label", "reacted_with:Mykhailo, Alice|👍");
    expect(screen.queryByTestId("reaction-chip-add")).not.toBeInTheDocument();
  });

  it("renders nothing read-only with no reactions", () => {
    const { container } = render(<ReactionChips reactions={[]} />);
    expect(container).toBeEmptyDOMElement();
  });
});
