// THREAT[TM-WEB-023]: tab-local recovery, never a source of authorization. Keys bind the account,
// organization and session; only server-uploaded attachment references are saved.
// Restored sends require explicit Retry: a reload cannot prove whether a request
// was accepted, nor whether the user still wants an old instruction executed.
import { isChatRetryExpired, type PendingSend } from "./chat-turn-state";

const PREFIX = "everruns:chat-recovery:v1:";
let epoch = 0;
export const chatRecoveryEpoch = () => epoch;
export const MAX_RECOVERY_BYTES = 1_048_576;
export const MAX_PENDING_SENDS = 50;

export function chatRecoveryKey(account: string, org: string, session: string): string {
  return PREFIX + JSON.stringify([account, org, session]);
}

interface Recovery {
  draft?: string;
  pending?: PendingSend[];
}

function read(key: string): Recovery {
  const raw = sessionStorage.getItem(key);
  if (!raw) return {};
  if (new Blob([raw]).size > MAX_RECOVERY_BYTES) throw new Error("recovery_size");
  const value = JSON.parse(raw);
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("recovery_format");
  return value;
}

export function readPendingSends(key?: string): PendingSend[] {
  if (!key) return [];
  const pending = read(key).pending ?? [];
  if (!Array.isArray(pending) || pending.length > MAX_PENDING_SENDS)
    throw new Error("recovery_format");
  return pending.map((item) => {
    if (
      !item ||
      typeof item.clientId !== "string" ||
      !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(item.clientId) ||
      typeof item.text !== "string" ||
      !Number.isFinite(item.sentAtMs) ||
      (item.createdAtMs !== undefined && !Number.isFinite(item.createdAtMs)) ||
      ![item.images, item.files].every((refs) => Array.isArray(refs) && refs.length <= 50) ||
      !item.images.every((ref) => ref && typeof ref.imageId === "string") ||
      !item.files.every((ref) => ref && typeof ref.fileId === "string")
    )
      throw new Error("recovery_format");
    return {
      ...item,
      createdAtMs: item.createdAtMs ?? item.sentAtMs,
      phase: "failed",
      error: undefined,
      retryWhenOnline: false,
      retryExpired: isChatRetryExpired(item),
    };
  });
}

export function readChatDraft(key?: string): string {
  if (!key) return "";
  const draft = read(key).draft;
  if (draft !== undefined && typeof draft !== "string") throw new Error("recovery_format");
  return draft ?? "";
}

export function writeChatRecovery(
  key: string | undefined,
  patch: Recovery,
  expectedEpoch?: number,
): void {
  if (!key) return;
  if (expectedEpoch !== undefined && expectedEpoch !== epoch)
    throw new Error("recovery_scope_retired");
  const value = { ...read(key), ...patch };
  // An acknowledged send is already durable on the server. Do not leave an
  // executable local copy after its acceptance.
  value.pending = value.pending?.filter((item) => item.phase !== "acked");
  if (!value.draft && !value.pending?.length) {
    sessionStorage.removeItem(key);
    return;
  }
  if ((value.pending?.length ?? 0) > MAX_PENDING_SENDS) throw new Error("recovery_size");
  const raw = JSON.stringify(value);
  if (new Blob([raw]).size > MAX_RECOVERY_BYTES) throw new Error("recovery_size");
  sessionStorage.setItem(key, raw);
}

export function clearChatRecovery(): void {
  epoch += 1;
  try {
    for (const key of Object.keys(sessionStorage)) {
      if (key.startsWith(PREFIX)) sessionStorage.removeItem(key);
    }
  } catch {
    // Storage may be disabled; logout must still complete.
  }
}
