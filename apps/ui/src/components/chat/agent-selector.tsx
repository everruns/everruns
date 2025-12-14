"use client";

import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Bot } from "lucide-react";
import type { Agent } from "@/lib/api/types";

interface AgentSelectorProps {
  agents: Agent[];
  selectedAgentId: string | null;
  onAgentChange: (agentId: string) => void;
  disabled?: boolean;
}

export function AgentSelector({
  agents,
  selectedAgentId,
  onAgentChange,
  disabled,
}: AgentSelectorProps) {
  return (
    <div className="flex items-center gap-3 p-4 border-b">
      <Bot className="h-5 w-5 text-muted-foreground" />
      <div className="flex-1 flex gap-3">
        <Select
          value={selectedAgentId || ""}
          onValueChange={onAgentChange}
          disabled={disabled}
        >
          <SelectTrigger className="w-64">
            <SelectValue placeholder="Select an agent" />
          </SelectTrigger>
          <SelectContent>
            {agents.map((agent) => (
              <SelectItem key={agent.id} value={agent.id}>
                {agent.name}
                <span className="text-muted-foreground ml-2 text-xs">
                  ({agent.default_model_id})
                </span>
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
    </div>
  );
}
