// THREAT[TM-WEB-023]: the host supplies a non-credential, login-scoped namespace.
// Restored requests need explicit Retry; never replay approvals, Stop or file edits.
export interface PendingMessage {
  key: string;
  body: unknown;
  createdAt: number;
  fromWidget?: boolean;
}
export interface Recovery {
  draft: string;
  pending?: PendingMessage;
}
const PREFIX = "everruns:work-chat:v1:";
export function recoveryKey(scope: string, session: string) {
  return PREFIX + JSON.stringify([scope, session]);
}
export function readRecovery(key: string): Recovery {
  const raw = sessionStorage.getItem(key);
  const draft = sessionStorage.getItem(key + ":draft");
  if ((raw && new Blob([raw]).size > 6 * 1024 * 1024) || (draft && draft.length > 1_048_576))
    throw new Error("Recovery storage is too large");
  const value = raw ? JSON.parse(raw) : { draft: "" };
  if (
    !value ||
    typeof value.draft !== "string" ||
    (value.pending &&
      (typeof value.pending.key !== "string" ||
        !Number.isFinite(value.pending.createdAt) ||
        !value.pending.body ||
        typeof value.pending.body !== "object"))
  )
    throw new Error("Invalid recovery storage");
  return draft === null ? value : { ...value, draft };
}
/** Draft keystrokes never rewrite the potentially multi-megabyte attachment body. */
export function saveDraft(key: string, draft: string) {
  if (draft.length > 1_048_576) throw new Error("Recovery storage is too large");
  sessionStorage.setItem(key + ":draft", draft);
}
export function saveRecovery(key: string, value: Recovery) {
  if (!value.draft && !value.pending) {
    sessionStorage.removeItem(key);
    sessionStorage.removeItem(key + ":draft");
    return;
  }
  const raw = JSON.stringify(value);
  if (new Blob([raw]).size > 6 * 1024 * 1024) throw new Error("Recovery storage is too large");
  sessionStorage.setItem(key, raw);
  sessionStorage.removeItem(key + ":draft");
}
export function clearRecovery() {
  try {
    for (const key of Object.keys(sessionStorage))
      if (key.startsWith(PREFIX)) sessionStorage.removeItem(key);
  } catch {
    /* logout still proceeds */
  }
}
export function retryable(pending: PendingMessage) {
  const age = Date.now() - pending.createdAt;
  return age >= 0 && age < 23 * 60 * 60 * 1000;
}
