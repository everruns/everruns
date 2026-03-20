"use client";

/**
 * Decisions:
 * - Keep todos in the transcript flow: one inline progress block, not a nested card with its own chrome.
 * - Prefer low-key motion: progress width changes and active-row pulse instead of a heavyweight widget.
 * - Reserve borders for the active item; avoid stacking box-on-box surfaces inside the chat transcript.
 */

import { useState } from "react";
import { CheckSquare2, ChevronDown, ChevronRight, Loader2, Square } from "lucide-react";
import { cn } from "@/lib/utils";
import type { ContentPart } from "@/lib/api/types";

// Re-export from centralized registry for backward-compatible imports.
export { isWriteTodosTool } from "@/lib/tool-registry";

// Todo item structure from write_todos tool
interface TodoItem {
  content: string;
  activeForm: string;
  status: "pending" | "in_progress" | "completed";
}

// Result structure from write_todos tool
interface WriteTodosResult {
  success: boolean;
  total_tasks: number;
  pending: number;
  in_progress: number;
  completed: number;
  todos: TodoItem[];
  warning?: string;
}

interface TodoListRendererProps {
  // For tool_call: the arguments passed to write_todos
  arguments?: Record<string, unknown>;
  // For tool_result: the result returned from write_todos
  result?: unknown;
  // Whether the tool is still executing (no result yet)
  isExecuting?: boolean;
  // Whether there was an error
  error?: string;
}

function extractResultObject(result: unknown): WriteTodosResult | null {
  if (!result) return null;

  if (typeof result === "object" && !Array.isArray(result)) {
    const candidate = result as WriteTodosResult;
    return Array.isArray(candidate.todos) ? candidate : null;
  }

  if (Array.isArray(result)) {
    const textParts = result
      .filter(
        (part): part is Extract<ContentPart, { type: "text" }> =>
          typeof part === "object" && part !== null && "type" in part && part.type === "text",
      )
      .map((part) => part.text);

    const text = textParts.join("\n").trim();
    if (!text) return null;

    try {
      const parsed = JSON.parse(text) as WriteTodosResult;
      return Array.isArray(parsed.todos) ? parsed : null;
    } catch {
      return null;
    }
  }

  return null;
}

function getStatusIcon(status: string, isActive: boolean = false) {
  switch (status) {
    case "completed":
      return <CheckSquare2 className="h-4 w-4 shrink-0 text-accent" />;
    case "in_progress":
      return <Loader2 className={cn("h-4 w-4 shrink-0 text-accent", isActive && "animate-spin")} />;
    case "pending":
    default:
      return <Square className="h-4 w-4 shrink-0 text-muted-foreground/35" />;
  }
}

function TodoItemRow({ todo, isActive }: { todo: TodoItem; isActive?: boolean }) {
  const isCompleted = todo.status === "completed";
  const isInProgress = todo.status === "in_progress";

  // Show activeForm when in_progress, otherwise show content
  const displayText = isInProgress ? todo.activeForm : todo.content;

  return (
    <div
      className={cn(
        "animate-chat-row-in flex items-start gap-2 py-1.5 transition-all duration-300",
        isInProgress && "border-l border-l-accent/70 bg-[hsl(var(--accent)/0.07)] pl-2.5",
        !isInProgress && "pl-[10px]",
      )}
    >
      {getStatusIcon(todo.status, isActive)}
      <span
        className={cn(
          "text-sm",
          isCompleted && "text-muted-foreground/60 line-through",
          isInProgress && "text-foreground",
          !isCompleted && !isInProgress && "text-muted-foreground/80",
        )}
      >
        {displayText}
      </span>
    </div>
  );
}

function TodoListFromItems({ todos, isActive }: { todos: TodoItem[]; isActive?: boolean }) {
  if (!todos || todos.length === 0) {
    return <div className="text-sm text-muted-foreground italic">No tasks</div>;
  }

  return (
    <div className="space-y-0.5">
      {todos.map((todo, index) => (
        <TodoItemRow
          key={`${todo.content}-${index}`}
          todo={todo}
          isActive={isActive && todo.status === "in_progress"}
        />
      ))}
    </div>
  );
}

export function TodoListRenderer({
  arguments: args,
  result,
  isExecuting,
  error,
}: TodoListRendererProps) {
  const [isExpanded, setIsExpanded] = useState(true);

  // Parse todos from arguments (tool_call) or result (tool_result)
  let todos: TodoItem[] = [];
  let parsedResult: WriteTodosResult | null = null;

  // Try to get todos from result first (it has the validated data)
  parsedResult = extractResultObject(result);
  if (parsedResult?.todos) {
    todos = parsedResult.todos;
  }

  // Fall back to arguments if no result yet (executing state)
  if (todos.length === 0 && args) {
    const todosList = args.todos;
    if (todosList && Array.isArray(todosList)) {
      todos = todosList as TodoItem[];
    }
  }

  // Handle error state
  if (error) {
    return <div className="text-xs text-red-600">Error: {error}</div>;
  }

  // Handle warning from result
  const warning = parsedResult?.warning;
  const completedCount = todos.filter((todo) => todo.status === "completed").length;
  const totalCount = todos.length;
  const progressPercent = totalCount === 0 ? 0 : Math.round((completedCount / totalCount) * 100);
  const activeTodo = todos.find((todo) => todo.status === "in_progress");

  return (
    <div className="space-y-2">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <div className="text-sm text-foreground">
              {completedCount} of {totalCount} todos completed
            </div>
            {isExecuting && (
              <span className="animate-tool-pulse inline-flex h-1.5 w-1.5 bg-accent" aria-hidden />
            )}
          </div>
          <div className="mt-0.5 text-xs text-muted-foreground/70">
            {activeTodo ? activeTodo.activeForm : "Plan captured from write_todos"}
          </div>
        </div>
        <button
          type="button"
          onClick={() => setIsExpanded((current) => !current)}
          className="rounded p-1 text-muted-foreground/60 transition-colors hover:bg-muted hover:text-foreground"
          aria-label={isExpanded ? "Collapse execution plan" : "Expand execution plan"}
        >
          {isExpanded ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
        </button>
      </div>

      <div className="h-1.5 overflow-hidden bg-muted/80">
        <div
          className={cn(
            "h-full bg-accent transition-[width] duration-500 ease-out",
            isExecuting && "animate-tool-progress-active",
          )}
          style={{ width: `${progressPercent}%` }}
        />
      </div>

      <div
        className={cn(
          "grid transition-all duration-300 ease-out",
          isExpanded ? "grid-rows-[1fr] opacity-100" : "grid-rows-[0fr] opacity-0",
        )}
      >
        <div className="min-h-0 overflow-hidden">
          <div className="space-y-1">
            <TodoListFromItems todos={todos} isActive={isExecuting} />
            {warning && <div className="mt-0.5 text-xs text-amber-600">{warning}</div>}
          </div>
        </div>
      </div>
    </div>
  );
}

// isWriteTodosTool is re-exported from @/lib/tool-registry at the top of this file.
