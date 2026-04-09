"use client";

import { useState, useCallback, useMemo } from "react";
import { useAgentExamples, useImportAgentExample, useCapabilities } from "@/hooks";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ArrowLeft, Search } from "lucide-react";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { ExampleCard } from "@/components/agents";

export default function AllExamplesPage() {
  const router = useRouter();
  const [exampleSearch, setExampleSearch] = useState("");
  const { data: allCapabilities } = useCapabilities();
  const { data: examples, isLoading, error } = useAgentExamples();
  const importExample = useImportAgentExample();
  const [importingName, setImportingName] = useState<string | null>(null);

  const handleImport = useCallback(
    async (name: string) => {
      setImportingName(name);
      try {
        const agent = await importExample.mutateAsync(name);
        router.push(`/agents/${agent.id}`);
      } catch (err) {
        console.error("Failed to import example:", err);
      } finally {
        setImportingName(null);
      }
    },
    [importExample, router],
  );

  const filteredExamples = useMemo(() => {
    if (!examples || !exampleSearch.trim()) return examples;
    const query = exampleSearch.toLowerCase();
    return examples.filter(
      (ex) =>
        ex.display_name.toLowerCase().includes(query) ||
        ex.name.toLowerCase().includes(query) ||
        ex.description.toLowerCase().includes(query) ||
        ex.tags.some((tag) => tag.toLowerCase().includes(query)),
    );
  }, [examples, exampleSearch]);

  return (
    <div className="container mx-auto p-6">
      <div className="flex items-center justify-between mb-6">
        <div className="flex items-center gap-3">
          <Link href="/agents">
            <Button variant="ghost" size="icon" className="h-8 w-8" aria-label="Back to agents">
              <ArrowLeft className="w-4 h-4" />
            </Button>
          </Link>
          <h1 className="text-2xl font-bold">Example Agents</h1>
        </div>
        <div className="relative">
          <Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            placeholder="Search examples..."
            value={exampleSearch}
            onChange={(e) => setExampleSearch(e.target.value)}
            className="pl-8 w-64"
          />
        </div>
      </div>

      <QueryStateWrapper
        isLoading={isLoading}
        error={error}
        data={filteredExamples}
        errorMessagePrefix="Failed to load examples"
        emptyState={
          <div className="text-center py-12">
            <p className="text-muted-foreground">
              {exampleSearch.trim() ? "No examples match your search" : "No examples available"}
            </p>
          </div>
        }
      >
        {(items) => (
          <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
            {items.map((example, index) => (
              <ExampleCard
                key={example.name ?? `example-${index}`}
                example={example}
                allCapabilities={allCapabilities}
                onImport={handleImport}
                adopting={importingName === example.name}
              />
            ))}
          </div>
        )}
      </QueryStateWrapper>
    </div>
  );
}
