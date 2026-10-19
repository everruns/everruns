"use client";

import { useState } from "react";
import { TaskCard } from "@/components/session/task-detail";
import { useCancelSessionTask, useSendTaskMessage } from "@/hooks/use-session-tasks";
import { Button, LinkButton } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";
import { isTerminalTaskState, type SessionTask } from "@/lib/api/types";

export function ChatThreadWorkDetail({ task }: { task: SessionTask }) {
  const [message, setMessage] = useState("");
  const send = useSendTaskMessage(task.session_id);
  const cancel = useCancelSessionTask(task.session_id);
  const terminal = isTerminalTaskState(task.state);
  // The task API reserves delegated conversation messages for the parent agent's tools.
  const canMessage = !terminal && !["subagent", "agent_handoff"].includes(task.kind);
  return (
    <div className="space-y-4 p-4">
      <TaskCard
        task={task}
        sessionId={task.session_id}
        inputReplyHint={
          canMessage ? "Answer below to continue." : "Ask Chat to answer this request."
        }
      />
      {task.links?.child_session_id && (
        <LinkButton href={`/sessions/${task.links.child_session_id}/trace`} variant="outline">
          Open session
        </LinkButton>
      )}
      {(send.error || cancel.error) && (
        <ChatErrorAlert message={(send.error ?? cancel.error)!.message} />
      )}
      {canMessage && (
        <form
          className="space-y-2"
          onSubmit={async (event) => {
            event.preventDefault();
            if (!message.trim() || send.isPending) return;
            try {
              await send.mutateAsync({
                taskId: task.id,
                request: {
                  content: [{ type: "text", text: message.trim() }],
                  ...(task.input_request ? { in_reply_to: task.input_request.id } : {}),
                },
              });
              setMessage("");
            } catch {
              /* Mutation exposes the error without losing the draft. */
            }
          }}
        >
          <label htmlFor="thread-work-message" className="text-sm font-medium">
            {task.input_request ? "Answer the request" : "Message this work"}
          </label>
          <Textarea
            id="thread-work-message"
            value={message}
            onChange={(e) => setMessage(e.target.value)}
          />
          <Button type="submit" disabled={!message.trim() || send.isPending}>
            Send
          </Button>
        </form>
      )}
      {!terminal && (
        <Button
          variant="outline"
          disabled={cancel.isPending || !!task.cancel_requested_at}
          onClick={() => cancel.mutate(task.id)}
        >
          {task.cancel_requested_at ? "Stopping…" : "Stop work"}
        </Button>
      )}
    </div>
  );
}
