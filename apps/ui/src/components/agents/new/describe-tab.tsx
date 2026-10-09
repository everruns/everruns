"use client";

import { useEffect, useRef, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { ArrowUp, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { draftAgent, type AgentDraft, type DraftTurn } from "@/lib/api/agent-draft";
import type { Capability } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { DraftCard } from "./draft-card";

const STARTER_PROMPTS = [
  "Chase overdue invoices every weekday and email the billing contact",
  "Answer HR policy questions in Slack",
  "Review pull requests and flag risky changes",
];

/** Describe the job to the agent builder; it edits the draft on the right. */
export function DescribeTab({
  draft,
  onDraftChange,
  messages,
  onMessagesChange,
  capabilities,
  orgName,
  creating,
  onCreate,
  onEdit,
}: {
  draft: AgentDraft;
  onDraftChange: (draft: AgentDraft) => void;
  messages: DraftTurn[];
  onMessagesChange: (messages: DraftTurn[]) => void;
  capabilities?: Capability[];
  orgName: string;
  creating: boolean;
  onCreate: () => void;
  onEdit: () => void;
}) {
  const [input, setInput] = useState("");
  const [suggestions, setSuggestions] = useState<string[]>(STARTER_PROMPTS);
  const scrollRef = useRef<HTMLDivElement>(null);
  const builder = useMutation({
    mutationFn: (conversation: DraftTurn[]) => draftAgent(conversation, draft),
    onSuccess: (result, conversation) => {
      onDraftChange(result.draft);
      onMessagesChange([...conversation, { role: "assistant", content: result.reply }]);
      setSuggestions(result.suggestions);
    },
  });

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [messages.length, builder.isPending]);

  const send = (text: string) => {
    const content = text.trim();
    if (!content || builder.isPending) return;
    const conversation: DraftTurn[] = [...messages, { role: "user", content }];
    onMessagesChange(conversation);
    setInput("");
    builder.mutate(conversation);
  };

  return (
    <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_320px]">
      <section className="flex min-h-[520px] flex-col rounded-md border bg-card">
        <header className="flex flex-wrap items-baseline gap-x-3 gap-y-1 border-b px-4 py-3">
          <h2 className="font-medium">Agent builder</h2>
          <p className="text-xs text-muted-foreground">
            Edits the draft on the right. Nothing is created until you confirm.
          </p>
        </header>
        <div ref={scrollRef} className="flex-1 space-y-3 overflow-y-auto px-4 py-4">
          <Bubble from="assistant">
            Describe the job in a sentence or two. I&apos;ll draft the agent on the right and pick
            from what {orgName} already has connected.
          </Bubble>
          {messages.map((message, index) => (
            <Bubble key={index} from={message.role}>
              {message.content}
            </Bubble>
          ))}
          {builder.isPending && (
            <Bubble from="assistant">
              <Loader2 className="size-4 animate-spin" aria-label="Drafting" />
            </Bubble>
          )}
          {builder.error && (
            <p role="alert" className="text-sm text-destructive">
              {builder.error.message}
            </p>
          )}
        </div>
        <div className="space-y-2 border-t px-4 py-3">
          {suggestions.length > 0 && (
            <div className="flex flex-wrap gap-2">
              {suggestions.map((suggestion) => (
                <Button
                  key={suggestion}
                  type="button"
                  variant="outline"
                  size="sm"
                  className="h-auto py-1 text-left whitespace-normal"
                  disabled={builder.isPending}
                  onClick={() => send(suggestion)}
                >
                  {suggestion}
                </Button>
              ))}
            </div>
          )}
          <form
            className="flex items-end gap-2"
            onSubmit={(event) => {
              event.preventDefault();
              send(input);
            }}
          >
            <Textarea
              value={input}
              onChange={(event) => setInput(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && !event.shiftKey) {
                  event.preventDefault();
                  send(input);
                }
              }}
              placeholder="What should this agent do?"
              aria-label="Describe the agent"
              rows={2}
              className="min-h-16 flex-1 resize-none"
            />
            <Button
              type="submit"
              variant="accent"
              size="icon"
              aria-label="Send"
              disabled={!input.trim() || builder.isPending}
            >
              <ArrowUp className="size-4" />
            </Button>
          </form>
        </div>
      </section>
      <DraftCard
        draft={draft}
        capabilities={capabilities}
        orgName={orgName}
        pending={creating}
        onCreate={onCreate}
        onEdit={onEdit}
      />
    </div>
  );
}

function Bubble({ from, children }: { from: DraftTurn["role"]; children: React.ReactNode }) {
  return (
    <div
      className={cn(
        "max-w-[85%] rounded-md px-3 py-2 text-sm whitespace-pre-wrap",
        from === "user" ? "ml-auto bg-accent/15" : "border bg-muted/40",
      )}
    >
      {children}
    </div>
  );
}
