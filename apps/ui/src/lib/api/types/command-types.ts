// Command types

import type { Controls } from "./message-types";

export type CommandSource = "system" | "skill";

export interface CommandArg {
  name: string;
  description: string;
  required: boolean;
}

export interface CommandDescriptor {
  name: string;
  description: string;
  source: CommandSource;
  args?: CommandArg[];
}

export interface CommandsResponse {
  commands: CommandDescriptor[];
}

export interface ExecuteCommandRequest {
  name: string;
  arguments?: string;
  controls?: Controls;
}

export interface CommandResult {
  success: boolean;
  message: string;
}
