"use client";

import { Circle, CircleDot, CheckCircle2, ListTodo } from "lucide-react";
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
      return <CheckCircle2 className="h-4 w-4 text-green-600 shrink-0" />;
    case "in_progress":
      return (
        <CircleDot
          className={cn(
            "h-4 w-4 text-blue-600 shrink-0",
            isActive && "animate-pulse"
          )}
        />
      );
    case "pending":
    default:
      return <Circle className="h-4 w-4 text-muted-foreground shrink-0" />;
  }
}

function TodoItemRow({ todo, isActive }: { todo: TodoItem; isActive?: boolean }) {
  const isCompleted = todo.status === "completed";
  const isInProgress = todo.status === "in_progress";

  // Show activeForm when in_progress, otherwise show content
  const displayText = isInProgress ? todo.activeForm : todo.content;

  return (
    <div className="flex items-start gap-2 py-0.5">
      {getStatusIcon(todo.status, isActive)}
      <span
        className={cn(
          "text-sm",
          isCompleted && "text-muted-foreground line-through",
          isInProgress && "font-medium text-foreground"
        )}
      >
        {displayText}
      </span>
    </div>
  );
}

function TodoListFromItems({ todos, isActive }: { todos: TodoItem[]; isActive?: boolean }) {
  if (!todos || todos.length === 0) {
    return (
      <div className="text-sm text-muted-foreground italic">
        No tasks
      </div>
    );
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

function TodoSummary({ result }: { result: WriteTodosResult }) {
  const { pending, in_progress, completed, total_tasks } = result;

  return (
    <div className="flex items-center gap-3 text-xs text-muted-foreground mt-1">
      <span>{total_tasks} task{total_tasks !== 1 ? "s" : ""}</span>
      {completed > 0 && (
        <span className="flex items-center gap-1">
          <CheckCircle2 className="h-3 w-3 text-green-600" />
          {completed}
        </span>
      )}
      {in_progress > 0 && (
        <span className="flex items-center gap-1">
          <CircleDot className="h-3 w-3 text-blue-600" />
          {in_progress}
        </span>
      )}
      {pending > 0 && (
        <span className="flex items-center gap-1">
          <Circle className="h-3 w-3" />
          {pending}
        </span>
      )}
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
    return (
      <div className="space-y-1">
        <div className="flex items-center gap-2">
          <ListTodo className="h-4 w-4 text-muted-foreground" />
          <span className="text-sm font-medium">Task List</span>
        </div>
        <div className="text-sm text-red-600">Error: {error}</div>
      </div>
    );
  }

  // Handle warning from result
  const warning = parsedResult?.warning;

  return (
    <div className="space-y-1">
      <div className="flex items-center gap-2">
        <ListTodo className="h-4 w-4 text-muted-foreground" />
        <span className="text-sm font-medium">Task List</span>
        {isExecuting && (
          <span className="text-xs text-muted-foreground">(updating...)</span>
        )}
      </div>
      <TodoListFromItems todos={todos} isActive={isExecuting} />
      {parsedResult && <TodoSummary result={parsedResult} />}
      {warning && (
        <div className="text-xs text-amber-600 mt-1">{warning}</div>
      )}
    </div>
  );
}

// Check if a tool call is for write_todos
export function isWriteTodosTool(toolName: string): boolean {
  return toolName === "write_todos";
}
