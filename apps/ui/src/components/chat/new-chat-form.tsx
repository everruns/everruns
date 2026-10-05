/**
 * New Playground chat: pick an Agent or harness, virtual user, and optional
 * Agent Sandbox before creating the session. Those bindings stay fixed
 * for the session lifetime (knowledge/ui/information-architecture.md).
 *
 * Personal Chats do not use this form. They always run the managed Platform
 * Chat Agent on its fixed runtime; Playground owns arbitrary Agent, harness,
 * identity, and Sandbox selection.
 *
 * An org with no model to chat with never reaches this form: both hosts own
 * their empty-state frame, so they swap the whole frame for the
 * "no intelligence available" message rather than stacking a second one here.
 */
"use client";

import { useState, type ReactNode } from "react";
import { useRouter } from "next/navigation";
import { Loader2, MessageCircle } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";
import { useAgents, useHarnesses } from "@/hooks";
import { useCreateSession } from "@/hooks/use-sessions";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { isHarnessDeprecated } from "@/lib/harness-deprecation";
import type { CreateSessionRequest } from "@/lib/api/types";

const AGENT_VALUE_PREFIX = "agent:";
const HARNESS_VALUE_PREFIX = "harness:";

export function NewPlaygroundChatForm({
  initialAgentId,
  endUserId,
  children,
}: {
  initialAgentId?: string;
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

  const start = async () => {
    if (!selection) return;
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
      <div className="flex flex-col gap-3">
        <Select value={selection} onValueChange={selectCounterpart} disabled={optionsLoading}>
          <SelectTrigger className="w-full" aria-label="Chat counterpart">
            <SelectValue
              placeholder={optionsLoading ? "Loading options..." : "Pick an agent or harness"}
            />
          </SelectTrigger>
          <SelectContent>
            {availableHarnesses.some(isHarnessDeprecated) && (
              <button
                type="button"
                className="w-full px-2 py-1.5 text-left text-sm text-muted-foreground"
                onClick={() => setShowDeprecated((shown) => !shown)}
                aria-pressed={showDeprecated}
              >
                {showDeprecated ? "Hide deprecated" : "Show deprecated"}
              </button>
            )}
            {harnesses.length > 0 && (
              <SelectGroup>
                <SelectLabel>Harnesses</SelectLabel>
                {harnesses.map((harness) => (
                  <SelectItem key={harness.id} value={`${HARNESS_VALUE_PREFIX}${harness.name}`}>
                    {getDisplayName(harness)}
                  </SelectItem>
                ))}
              </SelectGroup>
            )}
            {agents.length > 0 && (
              <SelectGroup>
                <SelectLabel>Agents</SelectLabel>
                {agents.map((agent) => (
                  <SelectItem key={agent.id} value={`${AGENT_VALUE_PREFIX}${agent.id}`}>
                    {getDisplayName(agent)}
                  </SelectItem>
                ))}
              </SelectGroup>
            )}
          </SelectContent>
        </Select>
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
        <Button
          variant="accent"
          onClick={start}
          disabled={!selection || createSession.isPending || !endUserId}
        >
          {createSession.isPending ? (
            <Loader2 className="size-4 animate-spin" />
          ) : (
            <MessageCircle className="size-4" />
          )}
          Start Playground chat
        </Button>
      </div>
      {error && <ChatErrorAlert message={error} />}
    </div>
  );
}
