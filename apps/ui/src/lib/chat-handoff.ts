// Tab-local, bounded draft handoff. No issue content enters a URL or is sent
// automatically. The existing recovery key binds account, organization and chat.
import { chatRecoveryEpoch } from "@/lib/chat-recovery";

export const CHAT_HANDOFF_EVENT = "everruns:chat-handoff";
const pending = new Map<string, { text: string; at: number; epoch: number }>();
const MAX_TEXT = 16_384;
const TTL = 5 * 60_000;

export function queueChatHandoff(key: string, text: string, epoch = chatRecoveryEpoch()) {
  if (epoch !== chatRecoveryEpoch()) throw new Error("Chat account changed");
  const previous = pending.get(key);
  const next =
    previous && Date.now() - previous.at < TTL && previous.epoch === epoch
      ? `${previous.text}\n\n${text}`
      : text;
  if (next.length > MAX_TEXT) throw new Error("Chat context is too large");
  if (pending.size >= 16 && !pending.has(key)) pending.delete(pending.keys().next().value!);
  pending.set(key, { text: next, at: Date.now(), epoch });
  window.dispatchEvent(new Event(CHAT_HANDOFF_EVENT));
}

export function takeChatHandoff(key?: string): string | undefined {
  if (!key) return;
  const value = pending.get(key);
  pending.delete(key);
  if (!value || value.epoch !== chatRecoveryEpoch() || Date.now() - value.at >= TTL) return;
  return value.text;
}
