import { HarnessIcon } from "@/lib/harness-icons";
import type { ConversationStarter } from "@/lib/api/legacy-api-types";
import { StreamdownMessage } from "./streamdown-message";

export interface PlatformChatIntroBoxProps {
  /** Display name shown in the intro card header (agent or harness). */
  title: string;
  /** Icon name for the intro card header (harness icon set). */
  icon?: string | null;
  /** Markdown intro (images allowed). Null hides the card body. */
  intro: string | null;
  /** Conversation starters rendered just above the composer. */
  starters: ConversationStarter[];
  /** Called with the starter text; the caller inserts it into the composer. */
  onSelect: (text: string) => void;
}

/**
 * Intro card + conversation starters for a fresh Platform Chat thread.
 * Rendered only while the transcript is empty; hidden once the user inputs.
 */
export function PlatformChatIntroBox({
  title,
  icon,
  intro,
  starters,
  onSelect,
}: PlatformChatIntroBoxProps) {
  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-3 px-4 pt-6">
      {intro ? (
        <div className="overflow-hidden rounded-2xl border bg-card shadow-xs">
          <div className="flex items-center gap-2.5 border-b px-4 py-3">
            <span className="flex size-8 items-center justify-center rounded-lg bg-primary/10">
              <HarnessIcon icon={icon} className="size-4.5 text-primary" />
            </span>
            <span className="text-sm font-semibold">{title}</span>
          </div>
          <StreamdownMessage variant="compact" className="px-4 py-3">
            {intro}
          </StreamdownMessage>
        </div>
      ) : null}
      {starters.length > 0 ? (
        <div
          className="grid grid-cols-1 gap-2 sm:grid-cols-2"
          role="list"
          aria-label="Conversation starters"
        >
          {starters.map((starter) => (
            <li key={starter.text} className="list-none">
              <button
                type="button"
                onClick={() => onSelect(starter.text)}
                className="group flex w-full items-center gap-2.5 rounded-xl border bg-card px-3.5 py-2.5 text-left text-sm shadow-xs transition-colors hover:border-primary/40 hover:bg-muted/60"
              >
                <span className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-muted transition-colors group-hover:bg-primary/10">
                  <HarnessIcon
                    icon={starter.icon}
                    className="size-4 text-muted-foreground transition-colors group-hover:text-primary"
                  />
                </span>
                <span className="line-clamp-2">{starter.text}</span>
              </button>
            </li>
          ))}
        </div>
      ) : null}
    </div>
  );
}
