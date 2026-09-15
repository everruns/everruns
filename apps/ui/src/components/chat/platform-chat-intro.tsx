import { HarnessIcon } from "@/lib/harness-icons";
import type { ConversationStarter } from "@/lib/api/legacy-api-types";
import { StreamdownMessage } from "./streamdown-message";

interface PlatformChatIntroBoxProps {
  /** Harness icon shown next to the intro line. */
  icon?: string | null;
  /** Intro markdown shown above the starters. */
  intro?: string | null;
  starters: ConversationStarter[];
  /** Called with the starter text; the caller inserts it into the composer. */
  onSelect: (text: string) => void;
}

/** Minimal first-run prompt for Platform Chat: one intro line plus plain starter rows. */
export function PlatformChatIntroBox({
  icon,
  intro,
  starters,
  onSelect,
}: PlatformChatIntroBoxProps) {
  if (!intro && starters.length === 0) {
    return null;
  }
  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-1 px-4 py-2">
      {intro ? (
        <div className="flex items-start gap-2.5 px-1 py-2">
          <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-primary/10">
            <HarnessIcon icon={icon} className="size-4 text-primary" />
          </span>
          <StreamdownMessage variant="compact" className="min-w-0 flex-1">
            {intro}
          </StreamdownMessage>
        </div>
      ) : null}
      {starters.length > 0 ? (
        <ul aria-label="Conversation starters" className="flex flex-col">
          {starters.map((starter) => (
            <li key={starter.text} className="list-none">
              <button
                type="button"
                onClick={() => onSelect(starter.text)}
                className="group flex w-full items-center gap-2.5 rounded-lg px-1 py-2 text-left text-sm hover:bg-muted/60"
              >
                <HarnessIcon
                  icon={starter.icon}
                  className="size-4 shrink-0 text-muted-foreground group-hover:text-primary"
                />
                <span>{starter.text}</span>
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}
