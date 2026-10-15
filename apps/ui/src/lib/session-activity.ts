// Keep opted-in event streams live for the entire mutation, including the gap
// between opening SSE and the server committing the input. Counts survive a
// subscriber mounting late or being temporarily disabled during the request.
type ActivityListener = (pending: number) => void;
const listeners = new Map<string, Set<ActivityListener>>();
const pendingActions = new Map<string, number>();

export function subscribeSessionActivity(
  sessionId: string,
  listener: ActivityListener,
): () => void {
  const group = listeners.get(sessionId) ?? new Set<ActivityListener>();
  group.add(listener);
  listeners.set(sessionId, group);
  const pending = pendingActions.get(sessionId) ?? 0;
  if (pending) listener(pending);
  return () => {
    group.delete(listener);
    if (group.size === 0) listeners.delete(sessionId);
  };
}

function updatePending(sessionId: string, delta: number) {
  const pending = (pendingActions.get(sessionId) ?? 0) + delta;
  if (pending) pendingActions.set(sessionId, pending);
  else pendingActions.delete(sessionId);
  for (const listener of listeners.get(sessionId) ?? []) listener(pending);
}

/** Wrap custom resume/write operations too when using followIdle: false. */
export async function withSessionActivity<T>(
  sessionId: string,
  action: () => Promise<T>,
): Promise<T> {
  updatePending(sessionId, 1);
  try {
    return await action();
  } finally {
    updatePending(sessionId, -1);
  }
}
