import type { MemoryEntry, MemoryStoreView } from "@/lib/api/types";

export function memoryStore(overrides: Partial<MemoryStoreView> = {}): MemoryStoreView {
  return {
    id: "mem_019dfb261a407c6085dcdd602402c3f7",
    name: "everruns-maintenance",
    description: null,
    scope: "org",
    source_type: "manual",
    source: { provider: "manual" },
    is_readonly: false,
    sync_status: "idle",
    last_synced_at: null,
    last_sync_error: null,
    status: "active",
    created_at: "2026-11-26T02:37:51Z",
    updated_at: "2026-11-26T02:37:51Z",
    archived_at: null,
    deleted_at: null,
    store: "everruns-maintenance",
    mount_path: "/memory/everruns-maintenance",
    about: "A named memory, shared by the agents it is attached to.",
    owner_name: null,
    entry_count: 3,
    file_count: 4,
    blocked_count: 1,
    proposal_count: 0,
    last_changed_at: "2026-11-26T05:00:00Z",
    ...overrides,
  };
}

export function memoryEntry(overrides: Partial<MemoryEntry> = {}): MemoryEntry {
  return {
    store: { scope: "team" },
    path: "/decisions/everruns-5528.md",
    kind: "decision",
    subject: "everruns#5528",
    statement: "Allowlist @a2ui/web_core",
    quote: "allow it",
    occurred_at: "2026-11-26T04:00:00Z",
    recorded_at: "2026-11-26T04:00:00Z",
    recorded_by: "agent:coordinator",
    ...overrides,
  };
}
