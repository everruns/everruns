// Adoption installs one observer for reviewed resource writes. It receives only
// method/path, never credentials, request bodies, response bodies or DOM state.
export interface ObservedWrite {
  headers?: Record<string, string>;
  succeeded: (committed?: {
    kind: string;
    id: string;
    revision: number;
    from_revision?: number;
  }) => Promise<void>;
  failed: () => void;
}
type Observer = (method: string, endpoint: string) => Promise<ObservedWrite | undefined>;
let active: { token: symbol; observer: Observer } | undefined;

export function observeResourceWrites(observer: Observer) {
  const token = Symbol();
  active = { token, observer };
  return () => {
    if (active?.token === token) active = undefined;
  };
}

export function beforeResourceWrite(method: string, endpoint: string) {
  return active?.observer(method, endpoint);
}

export interface ModelSelectionReceipt {
  id: string;
  from_revision: number;
  revision: number;
  before: boolean;
  after: boolean;
  label: string;
}
let selectionListener: ((items: ModelSelectionReceipt[]) => void) | undefined;
export function observeModelSelection(listener: (items: ModelSelectionReceipt[]) => void) {
  selectionListener = listener;
  return () => {
    if (selectionListener === listener) selectionListener = undefined;
  };
}
// Only this explicit public receipt is forwarded; generic API response bodies stay private.
export function recordModelSelection(items: ModelSelectionReceipt[]) {
  selectionListener?.(items);
}
