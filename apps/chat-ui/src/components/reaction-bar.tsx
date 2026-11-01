"use client";
import { useState } from "react";
import { QUICK_REACTIONS, type Reaction } from "./reactions";

/** Chips under a message, and a button that opens one quick row of emoji. */
export function ReactionBar({
  reactions,
  onToggle,
  alwaysShowButton,
}: {
  reactions: Reaction[];
  onToggle: (emoji: string, add: boolean) => void;
  /** The latest reply keeps its button visible (touch friendly). */
  alwaysShowButton: boolean;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div className={alwaysShowButton || reactions.length ? "reactions shown" : "reactions"}>
      {reactions.map((reaction) => (
        <button
          key={reaction.emoji}
          className={reaction.me ? "reaction mine" : "reaction"}
          aria-pressed={reaction.me}
          aria-label={`${reaction.emoji} ${reaction.count}${reaction.agent ? ", including the agent" : ""}`}
          onClick={() => onToggle(reaction.emoji, !reaction.me)}
        >
          <span>{reaction.emoji}</span> {reaction.count}
        </button>
      ))}
      <button
        className="reaction add"
        aria-label="Add reaction"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        ☺+
      </button>
      {open && (
        <span className="reaction-picker" role="menu">
          {QUICK_REACTIONS.map((emoji) => (
            <button
              key={emoji}
              role="menuitem"
              aria-label={emoji}
              onClick={() => {
                setOpen(false);
                const own = reactions.some((reaction) => reaction.emoji === emoji && reaction.me);
                onToggle(emoji, !own);
              }}
            >
              {emoji}
            </button>
          ))}
        </span>
      )}
    </div>
  );
}
