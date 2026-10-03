import type { ConversationStarter } from "./api/legacy-api-types";

/** Conversational presentation belongs to an Agent on every surface. */
export interface PlatformChatIntro {
  intro: string | null;
  description: string | null;
  starters: ConversationStarter[];
}
export interface IntroSource {
  intro_markdown?: string | null;
  short_description?: string | null;
  starters?: ConversationStarter[];
}
export function resolvePlatformChatIntro(agent: IntroSource | null | undefined): PlatformChatIntro {
  return {
    intro: agent?.intro_markdown || null,
    description: agent?.short_description || null,
    starters: agent?.starters ?? [],
  };
}
