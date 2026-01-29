"use client";

import { Circle, CircleDot } from "lucide-react";
import { cn } from "@/lib/utils";

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

function getStatusIcon(status: string, isActive: boolean = false) {
  switch (status) {
    case "completed":
      return <span className="text-green-600 text-xs shrink-0">✓</span>;
    case "in_progress":
      return (
        <CircleDot
          className={cn("h-3.5 w-3.5 text-blue-600 shrink-0", isActive && "animate-pulse")}
        />
      );
    case "pending":
    default:
      return <Circle className="h-3.5 w-3.5 text-muted-foreground/40 shrink-0" />;
  }
}

function TodoItemRow({ todo, isActive }: { todo: TodoItem; isActive?: boolean }) {
  const isCompleted = todo.status === "completed";
  const isInProgress = todo.status === "in_progress";

  // Show activeForm when in_progress, otherwise show content
  const displayText = isInProgress ? todo.activeForm : todo.content;

  return (
    <div className="flex items-start gap-1.5 py-0.5">
      {getStatusIcon(todo.status, isActive)}
      <span
        className={cn(
          "text-xs",
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
  // Parse todos from arguments (tool_call) or result (tool_result)
  let todos: TodoItem[] = [];
  let parsedResult: WriteTodosResult | null = null;

  // Try to get todos from result first (it has the validated data)
  if (result && typeof result === "object" && !Array.isArray(result)) {
    const resultObj = result as WriteTodosResult;
    if (resultObj.todos && Array.isArray(resultObj.todos)) {
      todos = resultObj.todos;
      parsedResult = resultObj;
    }
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

  return (
    <div className="space-y-0.5">
      <TodoListFromItems todos={todos} isActive={isExecuting} />
      {warning && <div className="text-xs text-amber-600 mt-0.5">{warning}</div>}
    </div>
  );
}

// Check if a tool call is for write_todos
export function isWriteTodosTool(toolName: string): boolean {
  return toolName === "write_todos";
}
