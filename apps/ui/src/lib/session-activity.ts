// Keep opted-in event streams live for the entire mutation, including the gap
// between opening SSE and the server committing the input. No global UI state.
type ActivityListener = () => () => void;
const listeners = new Map<string, Set<ActivityListener>>();

export function subscribeSessionActivity(
  sessionId: string,
  listener: ActivityListener,
): () => void {
  const group = listeners.get(sessionId) ?? new Set<ActivityListener>();
  group.add(listener);
  listeners.set(sessionId, group);
  return () => {
    group.delete(listener);
    if (group.size === 0) listeners.delete(sessionId);
  };
}

/** Wrap custom resume/write operations too when using followIdle: false. */
export async function withSessionActivity<T>(
  sessionId: string,
  action: () => Promise<T>,
): Promise<T> {
  const releases = [...(listeners.get(sessionId) ?? [])].map((listener) => listener());
  try {
    return await action();
  } finally {
    for (const release of releases) release();
  }
}
