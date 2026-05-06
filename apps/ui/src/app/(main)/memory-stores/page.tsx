"use client";

import { useState } from "react";
import { Brain, Plus, Search, Star, Trash2 } from "lucide-react";
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
import type { CreateMemoryStoreRequest, MemoryStore } from "@/lib/api/types";
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

  const { data: stores, isLoading, error } = useMemoryStores();
  const activeStoreId =
    selectedStoreId ?? stores?.find((s) => s.is_default)?.id ?? stores?.[0]?.id ?? null;

  const createStore = useCreateMemoryStore();
  const forgetMemory = useForgetMemory(activeStoreId ?? undefined);

  const memoriesQuery = useMemories(activeStoreId ?? undefined, {
    query: search,
    kind: kind === "all" ? undefined : kind,
    limit: 200,
  });

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

              <QueryStateWrapper
                isLoading={memoriesQuery.isLoading}
                error={memoriesQuery.error}
                data={memoriesQuery.data}
                errorMessagePrefix="Failed to load memories"
                skeletonCount={4}
                emptyState={<MemoriesEmptyState />}
              >
                {(response) => (
                  <div className="space-y-3">
                    <p className="text-xs text-muted-foreground">
                      {response.total} {response.total === 1 ? "memory" : "memories"}
                    </p>
                    {response.data.map((memory) => (
                      <MemoryCard
                        key={memory.id}
                        memory={memory}
                        onForget={() => forgetMemory.mutate(memory.id)}
                        forgetting={forgetMemory.isPending}
                      />
                    ))}
                  </div>
                )}
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
}: {
  memory: {
    id: string;
    content: string;
    kind: string;
    importance: number;
    tags: string[];
    active: boolean;
    created_at: string;
  };
  onForget: () => void;
  forgetting: boolean;
}) {
  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between gap-3 space-y-0">
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
        <Button
          variant="outline"
          size="sm"
          onClick={onForget}
          disabled={!memory.active || forgetting}
          aria-label="Forget memory"
        >
          <Trash2 className="h-4 w-4" />
          Forget
        </Button>
      </CardHeader>
      <CardContent className="space-y-2">
        <p className="text-sm">{memory.content}</p>
        <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <span className="truncate font-mono">{memory.id}</span>
          <CopyButton value={memory.id} />
          <span className="ml-auto">{formatRelativeTime(memory.created_at)}</span>
        </div>
      </CardContent>
    </Card>
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
