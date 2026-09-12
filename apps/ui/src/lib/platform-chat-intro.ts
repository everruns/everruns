import type { ConversationStarter } from "./api/legacy-api-types";
import { PLATFORM_CHAT_HARNESS_NAME } from "./chat-threads";

/** Resolved Platform Chat intro content for a thread. Agent values win. */
export interface PlatformChatIntro {
  /** Markdown intro (images allowed) for the intro box. Null when neither side sets one. */
  intro: string | null;
  /** One-line description in simplified Markdown for the header. Null when neither side sets one. */
  description: string | null;
  /** Conversation starters; the agent's win when non-empty. */
  starters: ConversationStarter[];
}

/**
 * Anything carrying Platform Chat intro content (agents and harnesses both
 * qualify structurally).
 */
export interface IntroSource {
  intro_markdown?: string | null;
  short_description?: string | null;
  starters?: ConversationStarter[];
}
type MaybeIntroSource = IntroSource | null | undefined;

/**
 * True when the thread is bound to the built-in Platform Chat harness.
 * Intro box, header description, and starters only render on these threads.
 */
export function isPlatformChatThread(harnessName: string | null | undefined): boolean {
  return harnessName === PLATFORM_CHAT_HARNESS_NAME;
}

function pickText(
  agent: MaybeIntroSource,
  harness: MaybeIntroSource,
  key: "intro_markdown" | "short_description",
): string | null {
  return agent?.[key] || harness?.[key] || null;
}

/**
 * Resolve intro content with per-field agent-wins semantics: the agent value
 * applies when set (non-empty), otherwise the harness value applies. Starters
 * come from the agent when it has any, otherwise from the harness.
 */
export function resolvePlatformChatIntro(
  agent: MaybeIntroSource,
  harness: MaybeIntroSource,
): PlatformChatIntro {
  const agentStarters = agent?.starters ?? [];
  return {
    intro: pickText(agent, harness, "intro_markdown"),
    description: pickText(agent, harness, "short_description"),
    starters: agentStarters.length > 0 ? agentStarters : [...(harness?.starters ?? [])],
  };
}
