// Command types

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
