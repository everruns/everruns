"use client";

import { useId, useState } from "react";
import { KeyRound, ShieldCheck } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { setSessionSecret, submitQuestionAnswers } from "@/lib/api/sessions";

/**
 * The browser half of an `ask_user` question with `kind: "secret"` (EVE-1058).
 *
 * Two posts, one card. The value goes to the session-secret endpoint, which has
 * always encrypted it; the question is then answered with a reference. The
 * question-answer endpoint is never handed the value, so there is no request
 * that could persist a credential as a tool result.
 *
 * It lives in its own file rather than as a branch of the choice card because
 * nothing here is shared with one: no options, no free text, no countdown, and
 * a field whose contents must never be echoed back.
 */
export interface AskUserSecretQuestion {
  id: string;
  header: string;
  question: string;
  secret_name: string;
  purpose: string;
}

interface AskUserSecretCardProps {
  sessionId: string;
  toolCallId: string;
  question: AskUserSecretQuestion;
  onResolved: (outcome: "answered" | "declined") => void;
}

export function AskUserSecretCard({
  sessionId,
  toolCallId,
  question,
  onResolved,
}: AskUserSecretCardProps) {
  const [value, setValue] = useState("");
  const [status, setStatus] = useState<"idle" | "submitting">("idle");
  const [error, setError] = useState<string | null>(null);
  const fieldId = useId();

  const submit = async () => {
    setStatus("submitting");
    setError(null);
    try {
      await setSessionSecret(sessionId, question.secret_name, value);
      await submitQuestionAnswers(sessionId, {
        tool_call_id: toolCallId,
        status: "answered",
        answers: [{ id: question.id, secret_ref: `session:${question.secret_name}` }],
      });
      // Drop the plaintext from component state the moment it is stored. It was
      // never sent anywhere else and it should not survive in memory either.
      setValue("");
      onResolved("answered");
    } catch {
      setStatus("idle");
      setError("Could not store the credential. Try again.");
    }
  };

  const decline = async () => {
    setStatus("submitting");
    setError(null);
    try {
      await submitQuestionAnswers(sessionId, {
        tool_call_id: toolCallId,
        status: "declined",
        answers: [],
      });
      setValue("");
      onResolved("declined");
    } catch {
      setStatus("idle");
      setError("Could not record your answer. Try again.");
    }
  };

  return (
    <form
      className="border border-border bg-muted/40"
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      <div className="flex items-start gap-3 border-b border-border px-4 py-3">
        <KeyRound className="mt-0.5 h-5 w-5 shrink-0 text-primary" />
        <div>
          <p className="text-sm font-medium text-foreground">The agent needs a credential</p>
          <p className="mt-0.5 text-xs text-muted-foreground">
            Stored encrypted on this session. The agent receives a reference, never the value.
          </p>
        </div>
      </div>

      <fieldset className="space-y-3 px-4 py-4">
        <div className="flex w-full items-center gap-2">
          <Badge variant="outline">{question.header}</Badge>
          <code className="text-[11px] text-muted-foreground">{question.secret_name}</code>
        </div>
        <label htmlFor={fieldId} className="block text-sm font-medium text-foreground">
          {question.question}
        </label>
        <p className="text-xs leading-5 text-muted-foreground">{question.purpose}</p>
        <Input
          id={fieldId}
          type="password"
          autoComplete="off"
          spellCheck={false}
          placeholder="Paste the credential"
          value={value}
          disabled={status === "submitting"}
          onChange={(event) => setValue(event.target.value)}
        />
      </fieldset>

      <div className="border-t border-border px-4 py-3">
        <p className="mb-3 flex items-center gap-1.5 text-xs text-muted-foreground">
          <ShieldCheck className="h-3.5 w-3.5" />
          This question does not time out. Nothing is answered for you.
        </p>
        {error && <p className="mb-3 text-xs text-destructive">{error}</p>}
        <div className="flex items-center justify-end gap-2">
          <Button
            type="button"
            size="sm"
            variant="ghost"
            className="text-muted-foreground"
            disabled={status === "submitting"}
            onClick={() => void decline()}
          >
            Decline
          </Button>
          <Button type="submit" size="sm" disabled={status === "submitting" || value.length === 0}>
            {status === "submitting" ? "Storing…" : "Store and continue"}
          </Button>
        </div>
      </div>
    </form>
  );
}
