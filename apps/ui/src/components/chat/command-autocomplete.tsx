"use client";

import { Terminal, Sparkles } from "lucide-react";
import type { CommandDescriptor } from "@/lib/api/types";
import { ComposerAutocomplete } from "@/components/chat/composer-autocomplete";

interface CommandAutocompleteProps {
  commands: CommandDescriptor[];
  inputValue: string;
  onSelect: (command: CommandDescriptor) => void;
  onDismiss: () => void;
  visible: boolean;
}

export function CommandAutocomplete({
  commands,
  inputValue,
  onSelect,
  onDismiss,
  visible,
}: CommandAutocompleteProps) {
  const query = inputValue.startsWith("/") ? inputValue.slice(1).toLowerCase() : "";
  const filtered = commands.filter(
    (cmd) =>
      cmd.name.toLowerCase().includes(query) || cmd.description.toLowerCase().includes(query),
  );

  return (
    <ComposerAutocomplete
      emptyMessage="No matching commands"
      items={filtered.map((command) => ({ ...command, id: command.name }))}
      label="Commands"
      onDismiss={onDismiss}
      onSelect={onSelect}
      visible={visible}
      renderItem={(cmd) => (
        <>
          {cmd.source === "system" ? (
            <Terminal className="icon-sharp h-3.5 w-3.5 flex-shrink-0 text-muted-foreground" />
          ) : (
            <Sparkles className="icon-sharp h-3.5 w-3.5 flex-shrink-0 text-muted-foreground" />
          )}
          <span className="font-mono text-xs text-foreground">/{cmd.name}</span>
          <span className="text-xs text-muted-foreground truncate">{cmd.description}</span>
        </>
      )}
    />
  );
}

/**
 * Returns whether the command autocomplete should be visible based on input.
 * Visible when: input starts with "/" and cursor is still in the command token
 * (no space after the slash-word).
 */
export function shouldShowCommandAutocomplete(input: string): boolean {
  return /^\/\S*$/.test(input);
}
