import type { FieldDiff } from "@/lib/api/types";
import { isSecretField } from "@/lib/api/change-history";
import { resourceKey, resourceWrite, type PageResource } from "./page-resource-history";
import type { ModelSelectionReceipt, ObservedWrite } from "@/lib/api/write-observer";

export interface ReviewForm {
  id: string;
  resource: PageResource;
  dirty: boolean;
  baseline: string;
}
interface FormState extends ReviewForm {
  revision?: number;
}
interface HistoryEntry {
  revision?: number | null;
  via_session_id?: string | null;
  action: string;
}
export interface SavedPageChange {
  resource: PageResource;
  from: number;
  revision: number;
  fields: FieldDiff[];
  stale: boolean;
  pending: boolean;
  error?: string;
  members?: ModelSelectionReceipt[];
}
interface ResourceState {
  resource: PageResource;
  revision?: number;
  loading?: Promise<void>;
  initializing?: boolean;
  error?: string;
  writes: number;
  reviewPending?: { from: number; revision: number };
  receipt?: SavedPageChange;
}
export interface ReviewTransport {
  history: (resource: PageResource) => Promise<HistoryEntry[]>;
  diff: (resource: PageResource, from: number, to: number) => Promise<FieldDiff[]>;
  restore: (
    resource: PageResource,
    from: number,
    revision: number,
    fields: string[],
  ) => Promise<number | undefined>;
  refresh: (resource: PageResource) => Promise<void>;
  currentOrg: () => string | null;
  undoSelection?: (
    items: { id: string; from_revision: number; expected_revision: number }[],
  ) => Promise<ModelSelectionReceipt[]>;
}

/** UI receipts contain sanitized history fields only. Drafts and credentials stay in their forms. */
export class PageChangeReviewController {
  private resources = new Map<string, ResourceState>();
  private forms = new Map<string, FormState>();
  private listeners = new Set<() => void>();
  private version = 0;
  private disposed = false;
  private epoch = 0;
  threadId?: string;
  activeResource?: PageResource;
  constructor(
    private org: string,
    private transport: ReviewTransport,
  ) {}
  get organizationId() {
    return this.org;
  }
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };
  snapshot = () => this.version;
  private emit() {
    if (!this.disposed) {
      this.version++;
      for (const listener of this.listeners) listener();
    }
  }
  private live(epoch = this.epoch) {
    return epoch === this.epoch && !this.disposed && this.transport.currentOrg() === this.org;
  }
  activate() {
    if (this.disposed) {
      this.epoch++;
      this.disposed = false;
    }
  }
  dispose() {
    this.epoch++;
    this.disposed = true;
    this.resources.clear();
    this.forms.clear();
    this.listeners.clear();
  }
  watch(resource: PageResource | undefined, threadId?: string) {
    const changed =
      (this.activeResource && resourceKey(this.activeResource)) !==
        (resource && resourceKey(resource)) || this.threadId !== threadId;
    this.activeResource = resource;
    this.threadId = threadId;
    if (changed) this.emit();
  }
  private prune() {
    const protectedKeys = new Set(
      [...this.forms.values()].map((form) => resourceKey(form.resource)),
    );
    if (this.activeResource) protectedKeys.add(resourceKey(this.activeResource));
    const inactive = [...this.resources.keys()].filter((key) => !protectedKeys.has(key));
    while (inactive.length > 20) {
      const key = inactive.shift()!;
      const state = this.resources.get(key)!;
      if (!state.loading && !state.writes && !state.receipt?.pending) this.resources.delete(key);
    }
  }
  private state(resource: PageResource) {
    const key = resourceKey(resource);
    let state = this.resources.get(key);
    if (!state) {
      state = { resource, writes: 0 };
      this.resources.set(key, state);
    }
    return state;
  }
  connect(form: ReviewForm) {
    this.activate();
    const state = this.state(form.resource);
    this.forms.set(form.id, { ...form, revision: state.revision });
    void this.load(state);
    this.emit();
    return () => {
      this.forms.delete(form.id);
      this.prune();
      this.emit();
    };
  }
  update(form: ReviewForm) {
    const previous = this.forms.get(form.id);
    if (!previous) return;
    if (previous.dirty === form.dirty && previous.baseline === form.baseline) return;
    const state = this.state(form.resource);
    this.forms.set(form.id, {
      ...form,
      revision: !previous.dirty || !form.dirty ? state.revision : previous.revision,
    });
    this.emit();
  }
  guard(formId: string) {
    const form = this.forms.get(formId);
    if (!form) return { loading: true, conflict: false };
    const state = this.state(form.resource);
    return {
      loading: state.revision === undefined || !!state.initializing,
      conflict: form.dirty && form.revision !== undefined && state.revision !== form.revision,
      error: state.error,
    };
  }
  receipts() {
    return [...this.resources.values()].flatMap((state) => (state.receipt ? [state.receipt] : []));
  }
  errors() {
    return [...this.resources.values()]
      .filter((state) => state.error)
      .map((state) => ({ resource: state.resource, error: state.error! }));
  }
  hasDirtyDraft(resource: PageResource) {
    return [...this.forms.values()].some(
      (form) => resourceKey(form.resource) === resourceKey(resource) && form.dirty,
    );
  }
  dismiss(resource: PageResource) {
    this.state(resource).receipt = undefined;
    this.emit();
  }
  async retry(resource: PageResource) {
    await this.load(this.state(resource));
  }
  async poll() {
    const watched = new Set([...this.forms.values()].map((form) => resourceKey(form.resource)));
    if (this.activeResource) {
      this.state(this.activeResource);
      watched.add(resourceKey(this.activeResource));
    }
    await Promise.all(
      [...this.resources.entries()]
        .filter(([key]) => watched.has(key))
        .map(([, state]) => this.load(state)),
    );
  }
  private async load(state: ResourceState) {
    if (state.loading) return state.loading;
    const epoch = this.epoch;
    state.initializing = state.revision === undefined;
    state.loading = (async () => {
      try {
        const entries = await this.transport.history(state.resource);
        if (!this.live(epoch)) return;
        const latest = entries.find((entry) => typeof entry.revision === "number");
        const revision = latest?.revision ?? 0;
        const previous = state.revision;
        state.revision = revision;
        if (state.reviewPending) {
          await this.makeReceipt(
            state,
            state.reviewPending.from || 1,
            state.reviewPending.revision,
            epoch,
          );
          if (!this.live(epoch)) return;
          state.reviewPending = undefined;
        }
        state.error = undefined;
        if (previous === undefined) {
          // Align the form's saved props with the history baseline before Save.
          await this.transport.refresh(state.resource);
          if (!this.live(epoch)) return;
          for (const form of this.forms.values())
            if (
              resourceKey(form.resource) === resourceKey(state.resource) &&
              form.revision === undefined
            )
              form.revision = revision;
        } else if (previous !== revision && !state.writes) {
          if (latest?.via_session_id === this.threadId && previous > 0 && this.threadId) {
            // Stop at the first foreign writer, including saves between polls.
            // A bounded history window also limits how far back Undo may reach.
            const between = entries.filter(
              (entry) => typeof entry.revision === "number" && entry.revision > previous,
            );
            const contiguous: number[] = [];
            for (const entry of between) {
              if (entry.via_session_id !== this.threadId) break;
              contiguous.push(entry.revision!);
            }
            const from = contiguous.length
              ? Math.max(previous, Math.min(...contiguous) - 1)
              : revision;
            if (state.receipt && state.receipt.revision !== from) state.receipt.stale = true;
            if (from < revision) await this.makeReceipt(state, from, revision, epoch);
          } else if (state.receipt && state.receipt.revision !== revision)
            state.receipt.stale = true;
          await this.transport.refresh(state.resource);
        }
      } catch (error) {
        if (this.live(epoch))
          state.error = error instanceof Error ? error.message : "Could not load change history.";
      } finally {
        if (this.live(epoch)) {
          state.initializing = false;
          state.loading = undefined;
          this.emit();
        }
      }
    })();
    return state.loading;
  }
  private async makeReceipt(
    state: ResourceState,
    from: number,
    revision: number,
    epoch = this.epoch,
  ) {
    const old = state.receipt;
    const baseline = old && !old.stale && old.revision === from ? old.from : from;
    const fields = (await this.transport.diff(state.resource, baseline, revision)).filter(
      (field) => !isSecretField(field.field),
    );
    if (!this.live(epoch)) return;
    state.receipt = fields.length
      ? { resource: state.resource, from: baseline, revision, fields, stale: false, pending: false }
      : undefined;
  }
  beforeWrite = async (method: string, endpoint: string): Promise<ObservedWrite | undefined> => {
    const epoch = this.epoch;
    const resource =
      endpoint === "/v1/users/me" && method.toUpperCase() === "PATCH"
        ? [...this.forms.values()].find((form) => form.resource.kind === "user_profile")?.resource
        : resourceWrite(method, endpoint, this.org);
    if (!resource || !this.live()) return;
    const state = this.state(resource);
    if (state.revision === undefined || state.initializing) await this.load(state);
    if (!this.live(epoch)) throw new Error("The organization changed. No changes were saved.");
    if (state.revision === undefined || state.error)
      throw new Error(
        "Could not verify the saved version. Your draft is kept; retry loading change history.",
      );
    const forms = [...this.forms.values()].filter(
      (form) => resourceKey(form.resource) === resourceKey(resource) && form.dirty,
    );
    const expected = forms.length
      ? Math.min(...forms.map((form) => form.revision ?? -1))
      : state.revision;
    if (expected < 0 || expected !== state.revision)
      throw new Error(
        "The saved version changed. Your draft is kept; discard it to review the latest version.",
      );
    state.writes++;
    this.emit();
    let settled = false;
    const finish = () => {
      if (!settled) {
        settled = true;
        state.writes--;
        this.emit();
      }
    };
    return {
      headers: {
        "Everruns-Expected-Change": JSON.stringify({
          kind: resource.kind,
          id: resource.id,
          revision: expected,
        }),
      },
      failed: finish,
      succeeded: async (committed) => {
        try {
          if (!committed || committed.id !== resource.id || committed.kind !== resource.kind)
            throw new Error("Missing committed revision");
          state.reviewPending = {
            from: committed.from_revision ?? expected,
            revision: committed.revision,
          };
          const entries = await this.transport.history(resource);
          if (!this.live(epoch)) return;
          const latest =
            entries.find((entry) => typeof entry.revision === "number")?.revision ?? expected;
          const revision = committed.revision;
          state.revision = latest;
          const from =
            committed.from_revision ??
            (expected || entries.find((entry) => entry.action === "baseline")?.revision);
          if (from && revision > from) await this.makeReceipt(state, from, revision, epoch);
          state.reviewPending = undefined;
          if (state.receipt && latest !== revision) state.receipt.stale = true;
          for (const form of forms) {
            const current = this.forms.get(form.id);
            if (current) current.revision = revision;
          }
          await this.transport.refresh(resource);
        } catch {
          if (this.live(epoch))
            state.error =
              "Saved successfully, but the change review could not load. Reload history; do not save again.";
        } finally {
          finish();
        }
      },
    };
  };
  recordSelection = (members: ModelSelectionReceipt[]) => {
    if (!members.length || !this.live()) return;
    const resource = {
      kind: "model_selection",
      id: `selection-${++this.version}`,
      label: "Model selection",
    };
    this.state(resource).receipt = {
      resource,
      from: 0,
      revision: 0,
      stale: false,
      pending: false,
      members,
      fields: members.map((item) => ({ field: item.id, from: item.before, to: item.after })),
    };
    this.prune();
    this.emit();
  };
  async undo(resource: PageResource, field?: string) {
    const epoch = this.epoch;
    const state = this.state(resource),
      receipt = state.receipt;
    if (
      !receipt ||
      receipt.pending ||
      receipt.stale ||
      this.hasDirtyDraft(resource) ||
      !this.live()
    )
      return;
    const fields = receipt.fields
      .filter((item) => !field || item.field === field)
      .map((item) => item.field);
    if (!fields.length) return;
    if (receipt.members) {
      if (!this.transport.undoSelection) return;
      receipt.pending = true;
      receipt.error = undefined;
      this.emit();
      try {
        const chosen = receipt.members.filter((item) => fields.includes(item.id));
        await this.transport.undoSelection(
          chosen.map((item) => ({
            id: item.id,
            from_revision: item.from_revision,
            expected_revision: item.revision,
          })),
        );
        if (!this.live(epoch)) return;
        receipt.members = receipt.members.filter((item) => !fields.includes(item.id));
        receipt.fields = receipt.fields.filter((item) => !fields.includes(item.field));
        if (!receipt.fields.length) state.receipt = undefined;
        await this.transport.refresh(resource);
      } catch (error) {
        if (this.live(epoch))
          receipt.error =
            error instanceof Error ? error.message : "Could not undo model selection.";
      } finally {
        receipt.pending = false;
        this.emit();
      }
      return;
    }
    await this.load(state);
    if (!this.live(epoch) || receipt.stale || state.error) return;
    receipt.pending = true;
    receipt.error = undefined;
    this.emit();
    let undone = false;
    try {
      const committed = await this.transport.restore(
        resource,
        receipt.from,
        receipt.revision,
        fields,
      );
      undone = true;
      if (!this.live(epoch)) return;
      if (committed === undefined)
        throw new Error(
          "The undo completed but its revision could not be verified. Reload history.",
        );
      receipt.revision = committed;
      receipt.fields = receipt.fields.filter((item) => !fields.includes(item.field));
      const entries = await this.transport.history(resource);
      if (!this.live(epoch)) return;
      const revision = entries.find((entry) => typeof entry.revision === "number")?.revision;
      if (!revision)
        throw new Error(
          "The undo completed but its history could not be verified. Reload history.",
        );
      state.revision = revision;
      receipt.stale = revision !== committed;
      if (!receipt.fields.length) state.receipt = undefined;
      await this.transport.refresh(resource);
    } catch (error) {
      if (this.live(epoch)) {
        if (undone) receipt.stale = true;
        receipt.error = error instanceof Error ? error.message : "Could not undo changes.";
      }
    } finally {
      receipt.pending = false;
      this.emit();
    }
  }
}
