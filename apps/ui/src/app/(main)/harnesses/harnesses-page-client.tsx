"use client";

import { useCallback, useState } from "react";
import {
  useCapabilities,
  useHarnessExamples,
  useHarnesses,
  useImportHarnessExample,
} from "@/hooks";
import { useRouter } from "next/navigation";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { ArrowRight, Plus } from "lucide-react";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { HarnessCard, HarnessExampleCard } from "@/components/harnesses";
import { ArchiveFilter } from "@/components/archive-filter";

const PREVIEW_LIMIT = 6;

export default function HarnessesPageClient() {
  const router = useRouter();
  const [showArchived, setShowArchived] = useState(false);
  const { data: harnesses, isLoading, error } = useHarnesses({ includeArchived: showArchived });
  const { data: allCapabilities } = useCapabilities();
  const { data: examples, isLoading: examplesLoading, error: examplesError } = useHarnessExamples();
  const importExample = useImportHarnessExample();
  const [importingName, setImportingName] = useState<string | null>(null);

  const handleImport = useCallback(
    async (name: string) => {
      setImportingName(name);
      try {
        const harness = await importExample.mutateAsync(name);
        router.push(`/harnesses/${harness.id}`);
      } catch (err) {
        console.error("Failed to import harness example:", err);
      } finally {
        setImportingName(null);
      }
    },
    [importExample, router],
  );

  const previewExamples = examples?.slice(0, PREVIEW_LIMIT);
  const hasMoreExamples = (examples?.length ?? 0) > PREVIEW_LIMIT;

  return (
    <div className="container mx-auto p-6 space-y-10">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">Harnesses</h1>
        <div className="flex items-center gap-2">
          <ArchiveFilter showArchived={showArchived} onShowArchivedChange={setShowArchived} />
          <Link href="/harnesses/new">
            <Button variant="accent">
              <Plus className="w-4 h-4 mr-2" />
              New Harness
            </Button>
          </Link>
        </div>
      </div>

      <section>
        <QueryStateWrapper
          isLoading={isLoading}
          error={error}
          data={harnesses}
          errorMessagePrefix="Failed to load harnesses"
          emptyState={
            <div className="text-center py-12">
              <p className="text-muted-foreground mb-4">No harnesses yet</p>
              <Link href="/harnesses/new">
                <Button>
                  <Plus className="w-4 h-4 mr-2" />
                  Create your first harness
                </Button>
              </Link>
            </div>
          }
        >
          {(items) => (
            <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
              {items.map((harness) => (
                <HarnessCard
                  key={harness.id}
                  harness={harness}
                  allCapabilities={allCapabilities}
                  showEditButton
                />
              ))}
            </div>
          )}
        </QueryStateWrapper>
      </section>

      <hr className="border-border" />

      <section>
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-xl font-semibold">Example harnesses</h2>
        </div>
        <QueryStateWrapper
          isLoading={examplesLoading}
          error={examplesError}
          data={previewExamples}
          errorMessagePrefix="Failed to load examples"
          emptyState={
            <div className="text-center py-12">
              <p className="text-muted-foreground">No examples available</p>
            </div>
          }
        >
          {(items) => (
            <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
              {items.map((example, index) => (
                <HarnessExampleCard
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
        {hasMoreExamples && (
          <div className="flex justify-end mt-4">
            <Link href="/harnesses/examples">
              <Button variant="outline">
                All examples
                <ArrowRight className="w-4 h-4 ml-2" />
              </Button>
            </Link>
          </div>
        )}
      </section>
    </div>
  );
}
