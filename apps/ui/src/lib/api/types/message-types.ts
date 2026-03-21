// Message types and content parts

// ============================================
// Message types (M2) - PRIMARY data
// ============================================

/**
 * Message role (API layer)
 *
 * Simplified to only user and agent messages.
 * Tool results are conveyed via `tool.completed` events.
 * System messages are internal and not exposed via API.
 */
export type MessageRole = "user" | "agent";

/**
 * Display message role (UI layer)
 *
 * Extended role type for rendering messages in the UI.
 * Includes "tool_result" for displaying tool execution results from events.
 */
export type DisplayMessageRole = MessageRole | "tool_result";

// ContentPart discriminated union - message content parts
export type ContentPart =
  | { type: "text"; text: string }
  | { type: "image"; url?: string; base64?: string; media_type?: string }
  | { type: "image_file"; image_id: string; filename?: string }
  | {
      type: "tool_call";
      id: string;
      name: string;
      arguments: Record<string, unknown>;
    }
  | {
      type: "tool_result";
      tool_call_id: string;
      result?: unknown;
      error?: string;
    };

// Helper type guards for ContentPart
export function isTextPart(part: ContentPart): part is { type: "text"; text: string } {
  return part.type === "text";
}

export function isToolCallPart(part: ContentPart): part is {
  type: "tool_call";
  id: string;
  name: string;
  arguments: Record<string, unknown>;
} {
  return part.type === "tool_call";
}

export function isToolResultPart(part: ContentPart): part is {
  type: "tool_result";
  tool_call_id: string;
  result?: unknown;
  error?: string;
} {
  return part.type === "tool_result";
}

export function isImageFilePart(
  part: ContentPart,
): part is { type: "image_file"; image_id: string; filename?: string } {
  return part.type === "image_file";
}

// Reasoning configuration for model controls
export interface ReasoningConfig {
  effort?: string;
}

// Runtime controls for message processing
// Model resolution priority: controls.model_id > session.model_id > agent.default_model_id > system default
export interface Controls {
  /** UUID of the model to use for this message (overrides session/agent settings) */
  model_id?: string;
  reasoning?: ReasoningConfig;
  max_tokens?: number;
  temperature?: number;
  /**
   * Generic client hints -- arbitrary key-value pairs declared by the client.
   * Session-level defaults are set at session creation; per-message values
   * override session hints key-by-key (shallow merge).
   *
   * Examples: `{ setup_connection: true, rich_media: true }`
   */
  hints?: Record<string, unknown>;
  /** Locale override for this message turn (BCP 47, e.g. `uk-UA`) */
  locale?: string;
}

/**
 * Message for UI display
 *
 * Uses DisplayMessageRole since messages can be derived from events
 * including tool.completed events which become "tool_result" messages.
 */
export interface Message {
  id: string;
  session_id: string;
  sequence: number;
  role: DisplayMessageRole;
  content: ContentPart[];
  metadata?: Record<string, unknown>;
  tool_call_id: string | null;
  /** Extended thinking content (Anthropic Claude with reasoning) */
  thinking?: string;
  /** Cryptographic signature for thinking (required for multi-turn) */
  thinking_signature?: string;
  created_at: string;
  /** Execution phase: "in_progress" (intermediate, has tool calls) or "completed" (final answer) */
  phase?: string;
}

// Message input for creating a message
export interface MessageInput {
  role: MessageRole;
  content: ContentPart[];
  metadata?: Record<string, unknown>;
  tool_call_id?: string;
}

// Request to create a message (new contract)
export interface CreateMessageRequest {
  message: MessageInput;
  controls?: Controls;
  metadata?: Record<string, unknown>;
  tags?: string[];
}

// Helper function to create a simple text message request
export function createTextMessageRequest(text: string, controls?: Controls): CreateMessageRequest {
  return {
    message: {
      role: "user",
      content: [{ type: "text", text }],
    },
    controls,
  };
}

// Helper function to extract text from content parts
export function getTextFromContent(content: ContentPart[]): string {
  return content
    .filter(isTextPart)
    .map((part) => part.text)
    .join("\n");
}

// Helper function to get tool calls from content parts
export function getToolCallsFromContent(
  content: ContentPart[],
): Array<{ id: string; name: string; arguments: Record<string, unknown> }> {
  return content.filter(isToolCallPart).map((part) => ({
    id: part.id,
    name: part.name,
    arguments: part.arguments,
  }));
}
