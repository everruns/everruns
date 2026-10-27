/**
 * New Playground chat: pick an Agent or harness, virtual user, and optional
 * Agent Sandbox before creating the session. Those bindings stay fixed
 * for the session lifetime (knowledge/ui/information-architecture.md).
 *
 * Personal Chats do not use this form. They always run the managed Platform
 * Chat Agent on its fixed runtime; Playground owns arbitrary Agent, harness,
 * identity, and Sandbox selection.
 *
 * Dry run (proactive agents D5) adds the `dry_run` session capability: tools
 * that only read run for real, every other call is simulated. The existing
 * checkbox selects this mode before starting a chat.
 *
 * An org with no model to chat with never reaches this form: both hosts own
 * their empty-state frame, so they swap the whole frame for the
 * "no intelligence available" message rather than stacking a second one here.
 */
"use client";

import { z } from "zod";
import { usePageReadContext } from "@/components/chat-mode/use-page-read-context";
import { usePageFormDraft } from "@/components/chat-mode/use-page-form-draft";
import { useState, type ReactNode } from "react";
import { useRouter } from "next/navigation";
import { Loader2, MessageCircle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Combobox } from "@/components/ui/combobox";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectLabel,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";
import { useAgents, useHarnesses } from "@/hooks";
import { useCreateSession } from "@/hooks/use-sessions";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { isHarnessDeprecated } from "@/lib/harness-deprecation";
import type { CreateSessionRequest } from "@/lib/api/types";

const playgroundDraftSchema = z.strictObject({
  counterpart: z.string(),
  sandbox: z.string(),
  dry_run: z.boolean(),
  end_user_id: z.string(),
});

const AGENT_VALUE_PREFIX = "agent:";
const HARNESS_VALUE_PREFIX = "harness:";

export function NewPlaygroundChatForm({
  initialAgentId,
  initialDryRun = false,
  endUserId,
  children,
}: {
  initialAgentId?: string;
  initialDryRun?: boolean;
  endUserId?: string;
  children?: ReactNode;
} = {}) {
  const router = useRouter();
  const { data: allAgents = [], isLoading: agentsLoading } = useAgents();
  const agents = allAgents.filter((a) => !(a.name === "platform-chat"));
  const { data: allHarnesses = [], isLoading: harnessesLoading } = useHarnesses();
  const availableHarnesses = allHarnesses.filter(
    (h) => !h.is_built_in || !h.name.startsWith("platform-chat"),
  );
  const createSession = useCreateSession();
  const [selection, setSelection] = useState(
    initialAgentId ? `${AGENT_VALUE_PREFIX}${initialAgentId}` : "",
  );
  const [showDeprecated, setShowDeprecated] = useState(false);
  const harnesses = availableHarnesses.filter(
    (harness) =>
      showDeprecated ||
      selection === `${HARNESS_VALUE_PREFIX}${harness.name}` ||
      !isHarnessDeprecated(harness),
  );
  const [sandbox, setSandbox] = useState("");
  const [dryRun, setDryRun] = useState(initialDryRun);
  const [error, setError] = useState<string | null>(null);
  const optionsLoading = agentsLoading || harnessesLoading;
  const selectedAgent = selection.startsWith(AGENT_VALUE_PREFIX)
    ? agents.find((agent) => agent.id === selection.slice(AGENT_VALUE_PREFIX.length))
    : undefined;
  const selectedHarness = selection.startsWith(HARNESS_VALUE_PREFIX)
    ? harnesses.find((harness) => harness.name === selection.slice(HARNESS_VALUE_PREFIX.length))
    : undefined;
  const usesManagedBashkitSandbox =
    selectedHarness?.name === "bashkit-worker" ||
    selectedAgent?.effective_harness?.name === "bashkit-worker";
  const sandboxTemplates = selectedAgent?.sandbox_policy?.templates ?? {};
  const sandboxNames = Object.keys(sandboxTemplates);
  const sandboxMode =
    selectedAgent?.sandbox_policy?.mode ?? (sandboxNames.length > 1 ? "selectable" : "fixed");
  const selectedSandbox = sandbox || selectedAgent?.sandbox_policy?.default || "";

  const selectCounterpart = (value: string) => {
    setSelection(value);
    const agent = value.startsWith(AGENT_VALUE_PREFIX)
      ? agents.find((candidate) => candidate.id === value.slice(AGENT_VALUE_PREFIX.length))
      : undefined;
    setSandbox(agent?.sandbox_policy?.default ?? "");
  };

  usePageReadContext({
    entityType: "playground_chat",
    label: "Playground choices",
    formId: "choices",
    fields: ["counterparts", "sandboxes", "choices_truncated"],
    values: {
      counterparts: JSON.stringify(
        [
          ...agents.map((a) => ({
            value: `${AGENT_VALUE_PREFIX}${a.id}`,
            name: getDisplayName(a).slice(0, 128),
          })),
          ...availableHarnesses.map((h) => ({
            value: `${HARNESS_VALUE_PREFIX}${h.name}`,
            name: getDisplayName(h).slice(0, 128),
          })),
        ].slice(0, 500),
      ),
      sandboxes: JSON.stringify(sandboxNames.slice(0, 100)),
      choices_truncated: String(
        agents.length + availableHarnesses.length > 500 || sandboxNames.length > 100,
      ),
    },
  });

  const chat = usePageFormDraft({
    schema: playgroundDraftSchema,
    entityType: "playground_chat",
    label: "New Playground chat",
    enabled: !optionsLoading,
    values: { counterpart: selection, sandbox, dry_run: dryRun, end_user_id: endUserId ?? "" },
    readonlyFields: ["end_user_id"],
    baseline: JSON.stringify([initialAgentId, initialDryRun, endUserId]),
    dirty: !!selection || dryRun !== initialDryRun || !!sandbox,
    writable: !createSession.isPending,
    apply: (next) => {
      selectCounterpart(next.counterpart);
      if (next.counterpart === selection || next.sandbox !== sandbox) setSandbox(next.sandbox);
      setDryRun(next.dry_run);
    },
  });

  const start = async () => {
    if (!selection || chat.blocked) return;
    if (!selectedAgent && !selectedHarness) {
      setError("Choose an available agent or harness.");
      return;
    }
    if (
      sandbox &&
      (!selectedAgent ||
        !sandboxNames.includes(sandbox) ||
        (sandboxMode === "fixed" && sandbox !== selectedAgent.sandbox_policy?.default))
    ) {
      setError("Choose a sandbox offered by this agent.");
      return;
    }
    setError(null);
    const binding: Partial<CreateSessionRequest> = selection.startsWith(HARNESS_VALUE_PREFIX)
      ? { harness_name: selection.slice(HARNESS_VALUE_PREFIX.length) }
      : { agent_id: selection.slice(AGENT_VALUE_PREFIX.length) };

    try {
      const session = await createSession.mutateAsync({
        request: {
          ...binding,
          ...(selectedAgent?.sandbox_policy && selectedSandbox && sandboxMode !== "fixed"
            ? { sandbox: { use: selectedSandbox } }
            : {}),
          ...(dryRun ? { capabilities: [{ ref: "dry_run", config: {} }] } : {}),
          source: "playground",
          playground_user_id: endUserId,
        },
      });
      router.push(`/playground/${session.id}`);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not start the chat.");
    }
  };

  if (!optionsLoading && agents.length === 0 && availableHarnesses.length === 0) {
    return (
      <div className="space-y-3">
        <p className="text-sm text-muted-foreground">
          There are no agents or harnesses available for a chat yet.
        </p>
        <Button onClick={() => router.push("/agents/new")}>Create an agent</Button>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {chat.notice}
      <div className="flex flex-col gap-3">
        <Combobox
          value={selection}
          onValueChange={selectCounterpart}
          ariaLabel="Chat counterpart"
          placeholder={optionsLoading ? "Loading options..." : "Pick an agent or harness"}
          searchPlaceholder="Search agents and harnesses"
          clearable={false}
          disabled={optionsLoading}
          options={[
            ...harnesses.map((harness) => ({
              value: `${HARNESS_VALUE_PREFIX}${harness.name}`,
              label: getDisplayName(harness),
              group: "Harnesses",
            })),
            ...agents.map((agent) => ({
              value: `${AGENT_VALUE_PREFIX}${agent.id}`,
              label: getDisplayName(agent),
              group: "Agents",
            })),
          ]}
          footer={
            availableHarnesses.some(isHarnessDeprecated) ? (
              <button
                type="button"
                className="w-full px-2 py-1.5 text-left text-sm text-muted-foreground"
                onClick={() => setShowDeprecated((shown) => !shown)}
                aria-pressed={showDeprecated}
              >
                {showDeprecated ? "Hide deprecated" : "Show deprecated"}
              </button>
            ) : undefined
          }
        />
        {selectedAgent?.sandbox_policy && sandboxNames.length > 0 && sandboxMode !== "fixed" ? (
          <Select value={selectedSandbox} onValueChange={setSandbox}>
            <SelectTrigger className="w-full" aria-label="Sandbox">
              <SelectValue placeholder="Pick a sandbox" />
            </SelectTrigger>
            <SelectContent>
              <SelectGroup>
                <SelectLabel>Runs in</SelectLabel>
                {sandboxNames.map((name) => {
                  const spec = sandboxTemplates[name];
                  const target = spec.target.provider || spec.target.kind;
                  return (
                    <SelectItem key={name} value={name}>
                      {name === selectedAgent.sandbox_policy?.default ? `${name} (default)` : name}{" "}
                      · {target}
                    </SelectItem>
                  );
                })}
              </SelectGroup>
            </SelectContent>
          </Select>
        ) : selectedAgent?.sandbox_policy && sandboxNames.length > 0 ? (
          <div className="border bg-muted/40 px-3 py-2 text-sm">
            <span className="text-muted-foreground">Sandbox: </span>
            {selectedSandbox} · fixed by Agent
          </div>
        ) : usesManagedBashkitSandbox ? (
          <div className="border bg-muted/40 px-3 py-2 text-sm">
            <span className="text-muted-foreground">Sandbox: </span>
            Bashkit Virtual Workspace · fixed by Bashkit Worker
          </div>
        ) : null}
        {children}
        <div className="flex items-start gap-2">
          <Checkbox id="dry-run" checked={dryRun} onCheckedChange={setDryRun} className="mt-0.5" />
          <label htmlFor="dry-run" className="text-sm">
            <span className="font-medium">Dry run</span>
            <span className="block text-xs text-muted-foreground">
              Tools that only read run for real. Every other action is simulated and changes
              nothing.
            </span>
          </label>
        </div>
        <Button
          variant="accent"
          onClick={start}
          disabled={chat.blocked || !selection || createSession.isPending || !endUserId}
        >
          {createSession.isPending ? (
            <Loader2 className="size-4 animate-spin" />
          ) : (
            <MessageCircle className="size-4" />
          )}
          {dryRun ? "Start test run" : "Start Playground chat"}
        </Button>
      </div>
      {error && <ChatErrorAlert message={error} />}
    </div>
  );
}
