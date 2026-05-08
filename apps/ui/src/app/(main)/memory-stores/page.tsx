"use client";

import { useEffect, useMemo, useState } from "react";
import { Brain, Plus, Search, Star, Trash2, X } from "lucide-react";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CopyButton } from "@/components/ui/copy-button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Drawer,
  DrawerContent,
  DrawerDescription,
  DrawerFooter,
  DrawerHeader,
  DrawerTitle,
} from "@/components/ui/drawer";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  useCreateMemoryStore,
  useForgetMemory,
  useMemories,
  useMemoryStores,
  usePageTitle,
} from "@/hooks";
import type {
  CreateMemoryStoreRequest,
  Memory,
  MemoryContentPart,
  MemoryStore,
} from "@/lib/api/types";
import { formatRelativeTime } from "@/lib/formatting";

const KIND_OPTIONS = [
  { value: "all", label: "All kinds" },
  { value: "fact", label: "Fact" },
  { value: "preference", label: "Preference" },
  { value: "correction", label: "Correction" },
  { value: "procedure", label: "Procedure" },
  { value: "context", label: "Context" },
];

export default function MemoryStoresPage() {
  usePageTitle("Memory");
  const [createOpen, setCreateOpen] = useState(false);
  const [selectedStoreId, setSelectedStoreId] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [kind, setKind] = useState<string>("all");
  const [selectedTags, setSelectedTags] = useState<string[]>([]);
  const [detailMemoryId, setDetailMemoryId] = useState<string | null>(null);

  const { data: stores, isLoading, error } = useMemoryStores();
  const activeStoreId =
    selectedStoreId ?? stores?.find((s) => s.is_default)?.id ?? stores?.[0]?.id ?? null;
  const activeStore = stores?.find((s) => s.id === activeStoreId) ?? null;

  const createStore = useCreateMemoryStore();
  const forgetMemory = useForgetMemory(activeStoreId ?? undefined);

  const memoriesQuery = useMemories(activeStoreId ?? undefined, {
    query: search,
    kind: kind === "all" ? undefined : kind,
    tag: selectedTags.length > 0 ? selectedTags : undefined,
    limit: 200,
  });

  // Tag chip set: union of tags present in the current results and tags
  // already selected, so a chip you picked doesn't disappear when the
  // narrowed result no longer contains it (you can still deselect it).
  const availableTags = useMemo(() => {
    const set = new Set<string>(selectedTags);
    for (const m of memoriesQuery.data?.data ?? []) {
      for (const t of m.tags) set.add(t);
    }
    return Array.from(set).sort();
  }, [memoriesQuery.data?.data, selectedTags]);

  const toggleTag = (tag: string) => {
    setSelectedTags((prev) =>
      prev.includes(tag) ? prev.filter((t) => t !== tag) : [...prev, tag].sort(),
    );
  };

  // Close the detail drawer when the memory list it was selected from changes,
  // so a previously-selected memory can't render against the wrong store/forget mutation.
  useEffect(() => {
    setDetailMemoryId(null);
  }, [activeStoreId, search, kind, selectedTags]);

  // Reset tag selection on store switch — tags are scoped to a store.
  useEffect(() => {
    setSelectedTags([]);
  }, [activeStoreId]);

  return (
    <div className="container mx-auto p-6">
      <div className="mb-6 flex flex-col gap-4 md:flex-row md:items-center md:justify-between">
        <h1 className="flex items-center gap-3 text-2xl font-bold">
          <Brain className="h-6 w-6" /> Memory
        </h1>
        <Button variant="accent" onClick={() => setCreateOpen(true)}>
          <Plus className="h-4 w-4" />
          New Store
        </Button>
      </div>

      <QueryStateWrapper
        isLoading={isLoading}
        error={error}
        data={stores}
        errorMessagePrefix="Failed to load memory stores"
        skeletonCount={3}
        emptyState={<EmptyState onCreate={() => setCreateOpen(true)} />}
      >
        {(items) => (
          <div className="grid gap-6 lg:grid-cols-[18rem_1fr]">
            <aside className="flex flex-col gap-2">
              {items.map((store) => (
                <StoreCard
                  key={store.id}
                  store={store}
                  active={store.id === activeStoreId}
                  onSelect={() => setSelectedStoreId(store.id)}
                />
              ))}
            </aside>
            <section className="space-y-4">
              <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
                <div className="relative sm:w-72">
                  <Search className="pointer-events-none absolute left-2.5 top-2 h-4 w-4 text-muted-foreground" />
                  <Input
                    value={search}
                    onChange={(event) => setSearch(event.target.value)}
                    placeholder="Search memory content"
                    className="pl-8"
                    aria-label="Search memories"
                  />
                </div>
                <Select value={kind} onValueChange={setKind}>
                  <SelectTrigger className="w-full sm:w-44">
                    <SelectValue placeholder="Kind" />
                  </SelectTrigger>
                  <SelectContent>
                    {KIND_OPTIONS.map((option) => (
                      <SelectItem key={option.value} value={option.value}>
                        {option.label}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              {availableTags.length > 0 && (
                <TagFilter
                  tags={availableTags}
                  selected={selectedTags}
                  onToggle={toggleTag}
                  onClear={() => setSelectedTags([])}
                />
              )}

              <QueryStateWrapper
                isLoading={memoriesQuery.isLoading}
                error={memoriesQuery.error}
                data={memoriesQuery.data?.data}
                errorMessagePrefix="Failed to load memories"
                skeletonCount={4}
                emptyState={<MemoriesEmptyState />}
              >
                {(items) => {
                  const total = memoriesQuery.data?.total ?? items.length;
                  return (
                    <div className="space-y-3">
                      <p className="text-xs text-muted-foreground">
                        {total} {total === 1 ? "memory" : "memories"}
                      </p>
                      {items.map((memory) => (
                        <MemoryCard
                          key={memory.id}
                          memory={memory}
                          onForget={() => forgetMemory.mutate(memory.id)}
                          forgetting={forgetMemory.isPending}
                          onSelect={() => setDetailMemoryId(memory.id)}
                          selectedTags={selectedTags}
                          onToggleTag={toggleTag}
                        />
                      ))}
                    </div>
                  );
                }}
              </QueryStateWrapper>
            </section>
          </div>
        )}
      </QueryStateWrapper>

      <CreateStoreDialog
        open={createOpen}
        isPending={createStore.isPending}
        onOpenChange={setCreateOpen}
        onSubmit={async (request) => {
          const created = await createStore.mutateAsync(request);
          setSelectedStoreId(created.id);
          setCreateOpen(false);
        }}
      />

      <MemoryDetailDrawer
        memory={
          detailMemoryId
            ? (memoriesQuery.data?.data.find((m) => m.id === detailMemoryId) ?? null)
            : null
        }
        store={activeStore}
        open={detailMemoryId !== null}
        onOpenChange={(open) => {
          if (!open) setDetailMemoryId(null);
        }}
        onForget={(id) => {
          forgetMemory.mutate(id, {
            onSuccess: () => setDetailMemoryId(null),
          });
        }}
        forgetting={forgetMemory.isPending}
      />
    </div>
  );
}

function StoreCard({
  store,
  active,
  onSelect,
}: {
  store: MemoryStore;
  active: boolean;
  onSelect: () => void;
}) {
  return (
    <Card
      role="button"
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
      className={`cursor-pointer transition ${active ? "border-primary" : ""}`}
    >
      <CardHeader className="space-y-2">
        <div className="flex items-start justify-between gap-2">
          <CardTitle className="truncate text-base">{store.name}</CardTitle>
          {store.is_default && (
            <Badge variant="secondary">
              <Star className="h-3 w-3" /> Default
            </Badge>
          )}
        </div>
        <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <span className="truncate font-mono">{store.id}</span>
          <CopyButton value={store.id} />
        </div>
      </CardHeader>
      <CardContent className="text-xs text-muted-foreground">
        {store.active_memory_count} active {store.active_memory_count === 1 ? "memory" : "memories"}
      </CardContent>
    </Card>
  );
}

function MemoryCard({
  memory,
  onForget,
  forgetting,
  onSelect,
  selectedTags,
  onToggleTag,
}: {
  memory: Memory;
  onForget: () => void;
  forgetting: boolean;
  onSelect: () => void;
  selectedTags: string[];
  onToggleTag: (tag: string) => void;
}) {
  return (
    <Card
      role="button"
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onSelect();
        }
      }}
      className="cursor-pointer transition hover:border-primary"
    >
      <CardHeader className="flex flex-row items-start justify-between gap-3 space-y-0">
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="outline">{memory.kind}</Badge>
          <Badge variant="secondary">importance {memory.importance}</Badge>
          {!memory.active && <Badge variant="destructive">forgotten</Badge>}
          {memory.tags.map((tag) => {
            const active = selectedTags.includes(tag);
            return (
              <button
                key={tag}
                type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  onToggleTag(tag);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.stopPropagation();
                  }
                }}
                aria-pressed={active}
                aria-label={`${active ? "Remove" : "Add"} tag filter ${tag}`}
                className="cursor-pointer"
              >
                <Badge
                  variant={active ? "default" : "outline"}
                  className="text-xs hover:border-primary"
                >
                  {tag}
                </Badge>
              </button>
            );
          })}
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={(e) => {
            e.stopPropagation();
            onForget();
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.stopPropagation();
            }
          }}
          disabled={!memory.active || forgetting}
          aria-label="Forget memory"
        >
          <Trash2 className="h-4 w-4" />
          Forget
        </Button>
      </CardHeader>
      <CardContent className="space-y-2">
        <p className="text-sm">{memory.content}</p>
        <div
          role="presentation"
          onClick={(e) => e.stopPropagation()}
          onKeyDown={(e) => e.stopPropagation()}
          className="flex items-center gap-1.5 text-xs text-muted-foreground"
        >
          <span className="truncate font-mono">{memory.id}</span>
          <CopyButton value={memory.id} />
          <span className="ml-auto">{formatRelativeTime(memory.created_at)}</span>
        </div>
      </CardContent>
    </Card>
  );
}

function MemoryDetailDrawer({
  memory,
  store,
  open,
  onOpenChange,
  onForget,
  forgetting,
}: {
  memory: Memory | null;
  store: MemoryStore | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onForget: (memoryId: string) => void;
  forgetting: boolean;
}) {
  return (
    <Drawer open={open} onOpenChange={onOpenChange}>
      <DrawerContent>
        {memory ? (
          <>
            <DrawerHeader>
              <DrawerTitle>Memory detail</DrawerTitle>
              <DrawerDescription>Full content, metadata, and attached parts.</DrawerDescription>
            </DrawerHeader>
            <div className="flex flex-1 flex-col gap-4 overflow-y-auto">
              <div className="flex flex-wrap items-center gap-2">
                <Badge variant="outline">{memory.kind}</Badge>
                <Badge variant="secondary">importance {memory.importance}</Badge>
                {!memory.active && <Badge variant="destructive">forgotten</Badge>}
                {memory.tags.map((tag) => (
                  <Badge key={tag} variant="outline" className="text-xs">
                    {tag}
                  </Badge>
                ))}
              </div>

              <section className="space-y-1">
                <h4 className="text-xs font-medium uppercase text-muted-foreground">Content</h4>
                <p className="whitespace-pre-wrap text-sm">{memory.content}</p>
              </section>

              {Array.isArray(memory.content_parts) && memory.content_parts.length > 0 && (
                <section className="space-y-2">
                  <h4 className="text-xs font-medium uppercase text-muted-foreground">
                    Content parts
                  </h4>
                  <div className="space-y-2">
                    {memory.content_parts.map((part, index) => (
                      <MemoryContentPartView key={`${memory.id}-part-${index}`} part={part} />
                    ))}
                  </div>
                </section>
              )}

              <section className="space-y-1">
                <h4 className="text-xs font-medium uppercase text-muted-foreground">Memory ID</h4>
                <div className="flex items-center gap-1.5 text-xs">
                  <span className="truncate font-mono">{memory.id}</span>
                  <CopyButton value={memory.id} />
                </div>
              </section>

              {store && (
                <section className="space-y-1">
                  <h4 className="text-xs font-medium uppercase text-muted-foreground">Store</h4>
                  <div className="flex items-center gap-2 text-sm">
                    <span>{store.name}</span>
                    {store.is_default && (
                      <Badge variant="secondary">
                        <Star className="h-3 w-3" /> Default
                      </Badge>
                    )}
                  </div>
                  <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
                    <span className="truncate font-mono">{store.id}</span>
                    <CopyButton value={store.id} />
                  </div>
                </section>
              )}

              <section className="grid grid-cols-2 gap-4 text-xs text-muted-foreground">
                <div>
                  <h4 className="text-xs font-medium uppercase">Created</h4>
                  <p>{formatRelativeTime(memory.created_at)}</p>
                </div>
                <div>
                  <h4 className="text-xs font-medium uppercase">Updated</h4>
                  <p>{formatRelativeTime(memory.updated_at)}</p>
                </div>
              </section>
            </div>
            <DrawerFooter>
              <Button
                variant="outline"
                onClick={() => onForget(memory.id)}
                disabled={!memory.active || forgetting}
              >
                <Trash2 className="h-4 w-4" />
                Forget memory
              </Button>
            </DrawerFooter>
          </>
        ) : null}
      </DrawerContent>
    </Drawer>
  );
}

// Allowlist of safe raster MIME types for inline `<img>` rendering. SVG is
// intentionally excluded — it can execute script in the authenticated UI
// context. Hoisted to module scope so we don't allocate a Set per render.
const SAFE_MEMORY_IMAGE_MEDIA_TYPES = new Set([
  "image/png",
  "image/jpeg",
  "image/gif",
  "image/webp",
  "image/bmp",
  "image/x-icon",
  "image/vnd.microsoft.icon",
]);

function MemoryContentPartView({ part }: { part: MemoryContentPart }) {
  if (part.type === "text") {
    return (
      <div className="rounded-md border bg-muted/30 p-3 text-sm whitespace-pre-wrap">
        {part.text}
      </div>
    );
  }
  if (part.type === "image") {
    if (!SAFE_MEMORY_IMAGE_MEDIA_TYPES.has(part.media_type.toLowerCase())) {
      return (
        <div className="rounded-md border bg-muted/30 p-3 text-sm text-muted-foreground">
          Unsupported image media type: {part.media_type}
        </div>
      );
    }
    const src = `data:${part.media_type};base64,${part.base64}`;
    return (
      <div className="rounded-md border bg-muted/30 p-2">
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img
          src={src}
          alt={`Memory attachment (${part.media_type})`}
          className="max-h-80 max-w-full rounded object-contain"
        />
        <p className="mt-1 text-xs text-muted-foreground">{part.media_type}</p>
      </div>
    );
  }
  return (
    <pre className="rounded-md border bg-muted/30 p-2 text-xs">{JSON.stringify(part, null, 2)}</pre>
  );
}

function TagFilter({
  tags,
  selected,
  onToggle,
  onClear,
}: {
  tags: string[];
  selected: string[];
  onToggle: (tag: string) => void;
  onClear: () => void;
}) {
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <span className="text-xs font-medium text-muted-foreground">Tags:</span>
      {tags.map((tag) => {
        const active = selected.includes(tag);
        return (
          <button
            key={tag}
            type="button"
            onClick={() => onToggle(tag)}
            aria-pressed={active}
            aria-label={`${active ? "Remove" : "Add"} tag filter ${tag}`}
            className="cursor-pointer"
          >
            <Badge
              variant={active ? "default" : "outline"}
              className="text-xs hover:border-primary"
            >
              {tag}
            </Badge>
          </button>
        );
      })}
      {selected.length > 0 && (
        <Button
          variant="ghost"
          size="sm"
          onClick={onClear}
          aria-label="Clear tag filters"
          className="h-6 gap-1 px-2 text-xs"
        >
          <X className="h-3 w-3" />
          Clear
        </Button>
      )}
    </div>
  );
}

function EmptyState({ onCreate }: { onCreate: () => void }) {
  return (
    <div className="flex flex-col items-center justify-center py-12 text-center">
      <Brain className="mb-4 h-12 w-12 text-muted-foreground" />
      <h3 className="mb-2 text-lg font-semibold">No memory stores</h3>
      <p className="mb-4 max-w-sm text-sm text-muted-foreground">
        Memory stores hold facts, preferences, and corrections that agents can recall across
        sessions. The default store is created on first agent use.
      </p>
      <Button variant="accent" onClick={onCreate}>
        <Plus className="h-4 w-4" />
        New Store
      </Button>
    </div>
  );
}

function MemoriesEmptyState() {
  return (
    <div className="flex flex-col items-center justify-center py-10 text-center text-sm text-muted-foreground">
      No memories yet. Agents using the <code className="font-mono">memory</code> capability will
      populate this store as they learn.
    </div>
  );
}

function CreateStoreDialog({
  open,
  isPending,
  onOpenChange,
  onSubmit,
}: {
  open: boolean;
  isPending: boolean;
  onOpenChange: (open: boolean) => void;
  onSubmit: (request: CreateMemoryStoreRequest) => Promise<void>;
}) {
  const [name, setName] = useState("");
  const [isDefault, setIsDefault] = useState(false);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>New memory store</DialogTitle>
          <DialogDescription>
            Create a named container for agent memories. Use distinct stores to scope memory by team
            or project.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <div className="space-y-1">
            <label className="text-sm font-medium" htmlFor="memory-store-name">
              Name
            </label>
            <Input
              id="memory-store-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="team-knowledge"
            />
          </div>
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={isDefault}
              onChange={(e) => setIsDefault(e.target.checked)}
            />
            Make this the default store for the organization
          </label>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={isPending}>
            Cancel
          </Button>
          <Button
            variant="accent"
            disabled={!name.trim() || isPending}
            onClick={async () => {
              await onSubmit({ name: name.trim(), is_default: isDefault });
              setName("");
              setIsDefault(false);
            }}
          >
            Create
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
