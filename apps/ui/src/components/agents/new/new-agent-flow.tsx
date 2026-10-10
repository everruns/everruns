"use client";

// New agent page. Four ways to start, one result: a normal
// agent. Describe it is the default (design note "Describe first"); examples,
// a blank form and package import are one tab away. Describe and Blank share
// one draft, so "Edit as form" carries the builder's work over.

import { useCallback, useRef, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { Upload } from "lucide-react";
import { AgentImportDialog } from "@/components/agents/agent-import-dialog";
import { ExampleCard } from "@/components/agents";
import { AgentIcon } from "@/components/icons/facet-icons";
import { Button } from "@/components/ui/button";
import { CodeBlock } from "@/components/ui/code-block";
import {
  EmptyState,
  PageBreadcrumb,
  PageContainer,
  PageMasthead,
  SectionTabs,
} from "@/components/layout";
import {
  useAgentExamples,
  useAgents,
  useCapabilities,
  useImportAgentExample,
  usePageTitle,
} from "@/hooks";
import { importedExampleLanding } from "@/lib/agent-template-setup";
import { EMPTY_DRAFT, type AgentDraft, type DraftTurn } from "@/lib/api/agent-draft";
import { newAgentLanding, parseNewAgentTab, type NewAgentTab } from "@/lib/new-agent";
import { useOrg } from "@/providers/org-provider";
import { BlankTab } from "./blank-tab";
import { useCreateAgentFromDraft } from "./create-from-draft";
import { DescribeTab } from "./describe-tab";

export function NewAgentFlow() {
  usePageTitle("New agent", "Agents");
  const router = useRouter();
  const searchParams = useSearchParams();
  const tab = parseNewAgentTab(searchParams.get("tab"));
  const { currentOrg } = useOrg();
  const orgName = currentOrg?.name ?? "Organization";
  const { data: capabilities } = useCapabilities();
  const { data: examples } = useAgentExamples();

  const [draft, setDraft] = useState<AgentDraft>(EMPTY_DRAFT);
  const [messages, setMessages] = useState<DraftTurn[]>([]);
  const { create, pending, error } = useCreateAgentFromDraft();
  const [createError, setCreateError] = useState<string | null>(null);

  const setTab = useCallback(
    (next: NewAgentTab) => {
      const params = new URLSearchParams(searchParams.toString());
      if (next === "describe") params.delete("tab");
      else params.set("tab", next);
      const query = params.toString();
      router.replace(query ? `/agents/new?${query}` : "/agents/new", {
        scroll: false,
      });
    },
    [router, searchParams],
  );

  const createFromDraft = async () => {
    setCreateError(null);
    try {
      const { agentId, failedChannels } = await create(draft);
      // A way in that failed is not lost work: the agent's integrations tab is
      // where it gets added by hand, so land there instead of hiding it.
      router.push(
        failedChannels.length > 0
          ? `/agents/${agentId}?tab=integrations`
          : newAgentLanding(agentId, draft),
      );
    } catch (err) {
      setCreateError(err instanceof Error ? err.message : "The agent could not be created.");
    }
  };

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Agents", href: "/agents" }, { label: "New agent" }]} />
      <PageMasthead
        icon={<AgentIcon />}
        title="New agent"
        description="Every path creates a normal agent. Channels start as drafts and take no traffic until you publish them."
      />
      <SectionTabs
        value={tab}
        onValueChange={(value) => setTab(value as NewAgentTab)}
        aria-label="How to start"
        items={[
          { value: "describe", label: "Describe it" },
          {
            value: "example",
            label: "From an example",
            count: examples?.length,
          },
          { value: "blank", label: "Blank" },
          { value: "import", label: "Import" },
        ]}
      />
      {(createError || error) && (tab === "describe" || tab === "blank") && (
        <p role="alert" className="text-sm text-destructive">
          {createError ?? error?.message}
        </p>
      )}
      <div className="pt-2">
        {tab === "describe" && (
          <DescribeTab
            draft={draft}
            onDraftChange={setDraft}
            messages={messages}
            onMessagesChange={setMessages}
            capabilities={capabilities}
            orgName={orgName}
            creating={pending}
            onCreate={createFromDraft}
            onEdit={() => setTab("blank")}
          />
        )}
        {tab === "blank" && (
          <BlankTab
            draft={draft}
            onDraftChange={setDraft}
            orgName={orgName}
            creating={pending}
            onCreate={createFromDraft}
          />
        )}
        {tab === "example" && <ExamplesTab />}
        {tab === "import" && <ImportTab />}
      </div>
    </PageContainer>
  );
}

function ExamplesTab() {
  const router = useRouter();
  const { data: capabilities } = useCapabilities({ includeRetired: true });
  const { data: examples, isLoading } = useAgentExamples();
  const importExample = useImportAgentExample();
  const [adopting, setAdopting] = useState<string | null>(null);

  const adopt = async (name: string) => {
    setAdopting(name);
    try {
      const agent = await importExample.mutateAsync(name);
      router.push(importedExampleLanding(examples, name, agent.id));
    } catch (err) {
      console.error("Failed to adopt example:", err);
    } finally {
      setAdopting(null);
    }
  };

  if (isLoading) return null;
  if (!examples?.length) {
    return <EmptyState title="No examples" description="This deployment ships no examples." />;
  }
  return (
    <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
      {examples.map((example) => (
        <ExampleCard
          key={example.name}
          example={example}
          allCapabilities={capabilities}
          onImport={adopt}
          adopting={adopting === example.name}
        />
      ))}
    </div>
  );
}

function ImportTab() {
  const router = useRouter();
  const { data: agents } = useAgents({ includeArchived: true });
  const { data: capabilities } = useCapabilities({ includeRetired: true });
  const inputRef = useRef<HTMLInputElement>(null);
  const [file, setFile] = useState<File | null>(null);
  const [dragging, setDragging] = useState(false);

  return (
    <>
      <div
        role="button"
        tabIndex={0}
        aria-label="Choose an agent package"
        onClick={() => inputRef.current?.click()}
        onKeyDown={(event) => {
          if (event.key === "Enter" || event.key === " ") inputRef.current?.click();
        }}
        onDragOver={(event) => {
          event.preventDefault();
          setDragging(true);
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={(event) => {
          event.preventDefault();
          setDragging(false);
          const dropped = event.dataTransfer.files?.[0];
          if (dropped) setFile(dropped);
        }}
        className={
          "flex cursor-pointer flex-col items-center gap-3 rounded-md border border-dashed px-6 py-12 text-center transition-colors " +
          (dragging ? "border-accent bg-accent/10" : "hover:bg-muted/40")
        }
      >
        <Upload className="size-6 text-muted-foreground" />
        <div className="font-medium">Drop an agent package</div>
        <p className="max-w-md text-sm text-muted-foreground">
          A zip that starts at agent.toml, or a single agent file (.md, .toml, .yaml, .json). You
          review the contents before anything is created.
        </p>
        <Button type="button" variant="outline" size="sm">
          Choose file
        </Button>
        <input
          ref={inputRef}
          type="file"
          className="hidden"
          accept=".md,.toml,.yaml,.yml,.json,.zip"
          aria-label="Import agent file"
          onChange={(event) => {
            const chosen = event.target.files?.[0];
            if (chosen) setFile(chosen);
            event.target.value = "";
          }}
        />
      </div>
      <div className="mt-4 space-y-2">
        <p className="text-sm text-muted-foreground">Export one from another organization:</p>
        <CodeBlock
          samples={[
            {
              label: "CLI",
              language: "bash",
              code: "everruns agents export triage --format zip",
            },
          ]}
        />
      </div>
      {file && (
        <AgentImportDialog
          file={file}
          agents={agents ?? []}
          capabilities={capabilities ?? []}
          onClose={() => setFile(null)}
          onImported={(agent) => router.push(`/agents/${agent.id}`)}
        />
      )}
    </>
  );
}
