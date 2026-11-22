/**
 * A message the platform wrote into the chat: a schedule run, a trigger, a
 * task wake, a notice (knowledge/execution/message-origin.md).
 *
 * Decisions:
 * - A task wake that parses is a status line without the report
 *   (task-update-marker.tsx). The transcript drops progress wakes and wakes
 *   right after their own run card before reaching this component.
 * - Everything else is one folded automatic-update line.
 */
"use client";

import { useLocale } from "@/providers/locale-provider";
import { AutomaticUpdate } from "@/components/chat/automatic-update";
import { MessageInfoIcon } from "@/components/chat/message-info-icon";
import { ORIGIN_ICON } from "@/components/chat/message-origin-icon";
import { TaskUpdateMarker } from "@/components/chat/task-update-marker";
import type { Event, SessionTask } from "@/lib/api/types";
import type { TaskUpdate } from "@/lib/chat-thread-messages";
import { ORIGIN_LABEL_KEY, type MessageOrigin } from "@/lib/message-origin";

export function PlatformMessage({
  event,
  origin,
  text,
  taskUpdate,
  task,
}: {
  event: Event;
  origin: MessageOrigin;
  text: string;
  taskUpdate: TaskUpdate | null;
  task?: SessionTask;
}) {
  const { t } = useLocale();

  if (taskUpdate && taskUpdate.kind !== "message") {
    const { kind } = taskUpdate;
    return (
      <TaskUpdateMarker
        update={{ ...taskUpdate, kind }}
        task={task}
        trailing={<MessageInfoIcon event={event} rawText={text} />}
      />
    );
  }

  const run = origin.run;
  const detail =
    run?.number === undefined
      ? undefined
      : run.limit === undefined
        ? t("origin_run_number", { number: run.number })
        : t("origin_run", { number: run.number, limit: run.limit });
  return (
    <AutomaticUpdate
      label={t(ORIGIN_LABEL_KEY[origin.kind])}
      icon={ORIGIN_ICON[origin.kind]}
      headline={origin.name}
      detail={detail}
      body={text}
      trailing={<MessageInfoIcon event={event} />}
    />
  );
}
