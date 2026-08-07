export interface WebMcpToolAnnotations {
  readOnlyHint?: boolean;
  destructiveHint?: boolean;
  idempotentHint?: boolean;
  untrustedContentHint?: boolean;
}

export interface WebMcpToolDefinition {
  name: string;
  description: string;
  inputSchema?: Record<string, unknown>;
  annotations?: WebMcpToolAnnotations;
  execute: (input: Record<string, unknown>) => unknown | Promise<unknown>;
}

export interface WebMcpRegistrationOptions {
  signal?: AbortSignal;
  exposedTo?: string[];
}

export interface WebMcpModelContext extends EventTarget {
  registerTool(tool: WebMcpToolDefinition, options?: WebMcpRegistrationOptions): Promise<void>;
}

declare global {
  interface Document {
    modelContext?: WebMcpModelContext;
  }
}
