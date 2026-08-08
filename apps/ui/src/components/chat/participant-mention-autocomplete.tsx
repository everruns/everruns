"use client";

import { AtSign, Bot } from "lucide-react";
import { ComposerAutocomplete } from "@/components/chat/composer-autocomplete";

export interface ParticipantMentionOption {
  id: string;
  label: string;
  description?: string;
}

export interface MentionQuery {
  end: number;
  query: string;
  start: number;
}

export function getMentionQuery(value: string, cursor: number): MentionQuery | null {
  const beforeCursor = value.slice(0, cursor);
  const match = /(?:^|\s)@([^\s@]*)$/.exec(beforeCursor);
  if (!match) return null;
  const atIndex = beforeCursor.lastIndexOf("@");
  return { start: atIndex, end: cursor, query: match[1] };
}

export function ParticipantMentionAutocomplete({
  options,
  query,
  visible,
  onDismiss,
  onSelect,
}: {
  options: ParticipantMentionOption[];
  query: string;
  visible: boolean;
  onDismiss: () => void;
  onSelect: (option: ParticipantMentionOption) => void;
}) {
  const normalizedQuery = query.toLocaleLowerCase();
  const filtered = options.filter((option) =>
    `${option.label} ${option.description ?? ""}`.toLocaleLowerCase().includes(normalizedQuery),
  );
  const emptyMessage =
    options.length === 0 ? "No participants available" : "No matching participants";

  return (
    <ComposerAutocomplete
      emptyMessage={emptyMessage}
      items={filtered}
      label="Address a participant"
      onDismiss={onDismiss}
      onSelect={onSelect}
      visible={visible}
      renderItem={(option) => (
        <>
          <Bot className="icon-sharp h-4 w-4 shrink-0 text-muted-foreground" />
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-1 font-medium text-foreground">
              <AtSign className="h-3.5 w-3.5 shrink-0" />
              <span className="truncate">{option.label}</span>
            </div>
            {option.description && (
              <div className="truncate text-xs text-muted-foreground">{option.description}</div>
            )}
          </div>
        </>
      )}
    />
  );
}
