"use client";
import { useUiTranslations } from "@/providers/locale-provider";
import { useInfiniteQuery } from "@tanstack/react-query";
import { useOrg } from "@/providers/org-provider";
import { listKnowledge } from "@/lib/api/knowledge";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
export function KnowledgeConfigEditor({
  config,
  onChange,
  disabled,
}: {
  config: Record<string, unknown>;
  onChange: (c: Record<string, unknown>) => void;
  disabled?: boolean;
}) {
  const ui = useUiTranslations();
  const { currentOrg } = useOrg();
  const collections = useInfiniteQuery({
    queryKey: ["knowledge-picker", currentOrg?.public_id],
    queryFn: ({ pageParam }) => listKnowledge(pageParam),
    initialPageParam: "",
    getNextPageParam: (p) => p.next_cursor ?? undefined,
    enabled: !!currentOrg,
  });
  const selected = Array.isArray(config.collections)
    ? config.collections.filter((v): v is string => typeof v === "string")
    : [];
  return (
    <div className="space-y-4">
      <p className="text-xs text-muted-foreground">
        {ui(
          "Selected collections mount read-only. Unindexed files remain available to read and grep.",
        )}
      </p>
      {collections.error && (
        <p role="alert" className="text-sm text-destructive">
          {ui("Unable to load collections.")}
        </p>
      )}
      <div className="max-h-60 space-y-3 overflow-auto">
        {collections.data?.pages
          .flatMap((p) => p.data)
          .map((c) => (
            <label key={c.id} className="flex items-center gap-2 text-sm">
              <Checkbox
                disabled={
                  disabled ||
                  (selected.length >= 32 &&
                    !selected.includes(c.id) &&
                    !(c.name === "everruns-docs" && selected.includes("everruns-docs")))
                }
                checked={
                  selected.includes(c.id) ||
                  (c.name === "everruns-docs" && selected.includes("everruns-docs"))
                }
                onCheckedChange={(checked) =>
                  onChange({
                    ...config,
                    collections: checked
                      ? [...new Set([...selected, c.id])]
                      : selected.filter(
                          (v) =>
                            v !== c.id && !(c.name === "everruns-docs" && v === "everruns-docs"),
                        ),
                  })
                }
              />
              <span>{c.name}</span>
              <span className="ml-auto text-xs text-muted-foreground">
                {c.indexing === "off" ? ui("Files only") : c.indexing}
              </span>
            </label>
          ))}
      </div>
      {collections.hasNextPage && (
        <button
          type="button"
          className="text-xs underline"
          disabled={collections.isFetchingNextPage}
          onClick={() => collections.fetchNextPage()}
        >
          {ui("Load more collections")}
        </button>
      )}
      <label className="flex items-center gap-2 text-sm">
        <Checkbox
          disabled={disabled}
          checked={config.automatic === true}
          onCheckedChange={(value) => onChange({ ...config, automatic: !!value })}
        />
        {ui("Retrieve relevant context automatically")}
      </label>
      <div className="space-y-2">
        <Label htmlFor="knowledge-result-limit">{ui("Result limit")}</Label>
        <Input
          id="knowledge-result-limit"
          disabled={disabled}
          type="number"
          min={1}
          max={25}
          value={typeof config.limit === "number" ? config.limit : 8}
          onChange={(e) => onChange({ ...config, limit: Number(e.target.value) })}
        />
      </div>
    </div>
  );
}
