"use client";

import Link from "next/link";
import { useState } from "react";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Bot, ArrowLeft, ChevronDown, ChevronRight, Check, Loader2, Circle, CheckCircle2, CircleDot, ListTodo } from "lucide-react";
import { cn } from "@/lib/utils";

const isDev = process.env.NODE_ENV === "development";

// Tool call type
interface ToolCallData {
  name: string;
  args: string;
  status: "completed" | "executing";
  result?: string | string[];
  todos?: Array<{ content: string; status: string }>;
}

// Conversation item types
type UserItem = { role: "user"; content: string };
type AgentItem = { role: "agent"; content: string };
type ToolItem = { role: "tool"; calls: ToolCallData[] };
type ConversationItem = UserItem | AgentItem | ToolItem;

// Sample conversation data
const sampleConversation: ConversationItem[] = [
  { role: "user", content: "Can you list the files in my project and run the tests?" },
  { role: "agent", content: "I'll check the project structure and run the test suite for you." },
  {
    role: "tool",
    calls: [
      {
        name: "list_files",
        args: "path: /home/user/project",
        status: "completed" as const,
        result: ["src/", "src/main.rs", "Cargo.toml"],
      },
      {
        name: "bash",
        args: 'command: "cargo test"',
        status: "completed" as const,
        result: "running 24 tests\ntest storage::tests::test_create ... ok\ntest api::tests::test_health ... ok\n\ntest result: ok. 24 passed",
      },
    ],
  },
  { role: "agent", content: "Your project has 3 files and all 24 tests passed successfully." },
  { role: "user", content: "Great! Now create a todo list for the remaining tasks." },
  { role: "agent", content: "I'll create a task list to track the remaining work." },
  {
    role: "tool",
    calls: [
      {
        name: "write_todos",
        args: "",
        status: "completed" as const,
        todos: [
          { content: "Review code changes", status: "completed" },
          { content: "Run tests", status: "in_progress" },
          { content: "Update documentation", status: "pending" },
          { content: "Create pull request", status: "pending" },
        ],
      },
    ],
  },
  { role: "agent", content: "I've created a task list. The code review is done and I'm currently running the tests." },
];

// Tool call with executing state
const executingConversation: ConversationItem[] = [
  { role: "user", content: "Read the main configuration file" },
  { role: "agent", content: "Let me read that file for you." },
  {
    role: "tool",
    calls: [
      {
        name: "read_file",
        args: "path: /home/user/project/config.json",
        status: "executing",
      },
    ],
  },
];

// ============================================
// Current Style (Option A)
// ============================================

function CurrentUserMessage({ content }: { content: string }) {
  return (
    <div className="flex justify-end">
      <div className="max-w-[90%] bg-gray-500 text-white rounded-lg p-3">
        <p className="text-sm whitespace-pre-wrap">{content}</p>
      </div>
    </div>
  );
}

function CurrentAgentMessage({ content }: { content: string }) {
  return (
    <div className="flex justify-start">
      <div className="w-full bg-muted/60 rounded-lg p-3">
        <div className="flex items-start gap-2">
          <Bot className="w-4 h-4 mt-0.5 flex-shrink-0 text-muted-foreground" />
          <p className="text-sm whitespace-pre-wrap">{content}</p>
        </div>
      </div>
    </div>
  );
}

function CurrentToolCall({ call }: { call: ToolCallData }) {
  const [expanded, setExpanded] = useState(false);

  if (call.name === "write_todos" && "todos" in call) {
    return (
      <div className="pl-6 text-sm text-muted-foreground">
        <div className="flex items-center gap-2 mb-1">
          <ListTodo className="h-4 w-4" />
          <span className="font-medium">Tasks</span>
        </div>
        <div className="space-y-0.5 ml-6">
          {call.todos?.map((todo, i) => (
            <div key={i} className="flex items-center gap-2">
              {todo.status === "completed" && <CheckCircle2 className="h-3.5 w-3.5 text-green-600" />}
              {todo.status === "in_progress" && <CircleDot className="h-3.5 w-3.5 text-blue-600" />}
              {todo.status === "pending" && <Circle className="h-3.5 w-3.5 text-muted-foreground" />}
              <span className={cn(
                "text-sm",
                todo.status === "completed" && "text-muted-foreground line-through",
                todo.status === "in_progress" && "font-medium"
              )}>{todo.content}</span>
            </div>
          ))}
        </div>
      </div>
    );
  }

  return (
    <div className="pl-6 text-sm text-muted-foreground space-y-0.5">
      <div>
        <span className="font-medium">{call.name}:</span>
        {call.args && <span className="ml-1">{call.args}</span>}
      </div>
      {call.status === "executing" ? (
        <div>&gt; ... executing ...</div>
      ) : call.result ? (
        <div>
          <div className="whitespace-pre-wrap">&gt; {String(call.result).substring(0, 100)}{String(call.result).length > 100 ? "..." : ""}</div>
          {String(call.result).length > 100 && (
            <button onClick={() => setExpanded(!expanded)} className="text-xs text-blue-600 hover:underline">
              {expanded ? "show less" : "> see more..."}
            </button>
          )}
          {expanded && <pre className="text-xs bg-muted/50 p-2 rounded mt-1">{call.result}</pre>}
        </div>
      ) : null}
    </div>
  );
}

// ============================================
// Option B: Flat Agent, Boxed User
// ============================================

function FlatUserMessage({ content }: { content: string }) {
  return (
    <div className="flex justify-end">
      <div className="max-w-[85%] bg-primary text-primary-foreground rounded-2xl px-4 py-2.5">
        <p className="text-sm">{content}</p>
      </div>
    </div>
  );
}

function FlatAgentMessage({ content }: { content: string }) {
  return (
    <div className="flex justify-start">
      <div className="w-full">
        <p className="text-sm whitespace-pre-wrap">{content}</p>
      </div>
    </div>
  );
}

function FlatToolCall({ call }: { call: ToolCallData }) {
  const [expanded, setExpanded] = useState(false);

  if (call.name === "write_todos" && "todos" in call) {
    return (
      <div className="text-sm border-l-2 border-muted-foreground/20 pl-3 py-1">
        <div className="space-y-0.5">
          {call.todos?.map((todo, i) => (
            <div key={i} className="flex items-center gap-2">
              {todo.status === "completed" && <Check className="h-3.5 w-3.5 text-green-600" />}
              {todo.status === "in_progress" && <Loader2 className="h-3.5 w-3.5 text-blue-600 animate-spin" />}
              {todo.status === "pending" && <Circle className="h-3.5 w-3.5 text-muted-foreground/40" />}
              <span className={cn(
                todo.status === "completed" && "text-muted-foreground",
                todo.status === "in_progress" && "text-foreground",
                todo.status === "pending" && "text-muted-foreground"
              )}>{todo.content}</span>
            </div>
          ))}
        </div>
      </div>
    );
  }

  const statusIcon = call.status === "executing"
    ? <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />
    : <Check className="h-3.5 w-3.5 text-green-600" />;

  return (
    <div className="text-sm text-muted-foreground">
      <div className="flex items-center gap-1.5">
        {statusIcon}
        <span className="font-mono text-xs">{call.name}</span>
        {call.args && <span className="text-muted-foreground/60 text-xs truncate max-w-[300px]">{call.args}</span>}
      </div>
      {expanded && call.result && (
        <pre className="text-xs bg-muted/30 p-2 rounded mt-1 ml-5">{call.result}</pre>
      )}
      {call.result && String(call.result).length > 50 && (
        <button
          onClick={() => setExpanded(!expanded)}
          className="ml-5 text-xs text-muted-foreground/60 hover:text-muted-foreground flex items-center gap-0.5"
        >
          {expanded ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
          {expanded ? "hide" : "output"}
        </button>
      )}
    </div>
  );
}

// ============================================
// Option C: Minimal - subtle everything
// ============================================

function MinimalUserMessage({ content }: { content: string }) {
  return (
    <div className="flex justify-end">
      <div className="max-w-[85%] border border-border/60 rounded-xl px-3 py-2">
        <p className="text-sm">{content}</p>
      </div>
    </div>
  );
}

function MinimalAgentMessage({ content }: { content: string }) {
  return (
    <div className="flex justify-start pl-1">
      <p className="text-sm whitespace-pre-wrap text-foreground/90">{content}</p>
    </div>
  );
}

function MinimalToolCall({ call }: { call: ToolCallData }) {
  const [expanded, setExpanded] = useState(false);

  if (call.name === "write_todos" && "todos" in call) {
    return (
      <div className="text-xs text-muted-foreground/80 pl-1 space-y-0.5">
        {call.todos?.map((todo, i) => (
          <div key={i} className="flex items-center gap-1.5">
            {todo.status === "completed" && <span className="text-green-600">✓</span>}
            {todo.status === "in_progress" && <span className="text-blue-600">○</span>}
            {todo.status === "pending" && <span className="text-muted-foreground/40">○</span>}
            <span className={cn(
              todo.status === "completed" && "line-through opacity-60"
            )}>{todo.content}</span>
          </div>
        ))}
      </div>
    );
  }

  const status = call.status === "executing" ? "..." : "✓";

  return (
    <div className="text-xs text-muted-foreground/70 pl-1">
      <span className="inline-flex items-center gap-1">
        <span className={call.status === "executing" ? "text-muted-foreground" : "text-green-600"}>{status}</span>
        <span className="font-mono">{call.name}</span>
        {call.args && <span className="opacity-60">{call.args}</span>}
        {call.result && String(call.result).length > 30 && (
          <button onClick={() => setExpanded(!expanded)} className="opacity-50 hover:opacity-100">
            [{expanded ? "−" : "+"}]
          </button>
        )}
      </span>
      {expanded && call.result && (
        <pre className="mt-1 p-1.5 bg-muted/20 rounded text-[10px] leading-tight">{call.result}</pre>
      )}
    </div>
  );
}

// ============================================
// Option D: Claude-like flat with accent user
// ============================================

function ClaudeUserMessage({ content }: { content: string }) {
  return (
    <div className="flex justify-end">
      <div className="max-w-[85%] bg-orange-50 dark:bg-orange-950/30 rounded-2xl px-4 py-2.5 border border-orange-200/50 dark:border-orange-800/30">
        <p className="text-sm">{content}</p>
      </div>
    </div>
  );
}

function ClaudeAgentMessage({ content }: { content: string }) {
  return (
    <div className="flex justify-start gap-2">
      <div className="w-6 h-6 rounded-full bg-gradient-to-br from-orange-400 to-amber-500 flex items-center justify-center flex-shrink-0 mt-0.5">
        <Bot className="w-3.5 h-3.5 text-white" />
      </div>
      <div className="flex-1">
        <p className="text-sm whitespace-pre-wrap">{content}</p>
      </div>
    </div>
  );
}

function ClaudeToolCall({ call }: { call: ToolCallData }) {
  const [expanded, setExpanded] = useState(false);

  if (call.name === "write_todos" && "todos" in call) {
    return (
      <div className="ml-8 text-sm bg-muted/30 rounded-lg p-2.5">
        <div className="space-y-1">
          {call.todos?.map((todo, i) => (
            <div key={i} className="flex items-center gap-2">
              {todo.status === "completed" && <CheckCircle2 className="h-4 w-4 text-green-600" />}
              {todo.status === "in_progress" && <CircleDot className="h-4 w-4 text-orange-500 animate-pulse" />}
              {todo.status === "pending" && <Circle className="h-4 w-4 text-muted-foreground/40" />}
              <span className={cn(
                "text-sm",
                todo.status === "completed" && "text-muted-foreground line-through",
                todo.status === "in_progress" && "font-medium text-foreground"
              )}>{todo.content}</span>
            </div>
          ))}
        </div>
      </div>
    );
  }

  return (
    <div className="ml-8 text-sm">
      <div
        className={cn(
          "inline-flex items-center gap-1.5 px-2 py-1 rounded-md",
          call.status === "executing" ? "bg-orange-100 dark:bg-orange-950/40" : "bg-muted/40"
        )}
      >
        {call.status === "executing" ? (
          <Loader2 className="h-3 w-3 animate-spin text-orange-600" />
        ) : (
          <Check className="h-3 w-3 text-green-600" />
        )}
        <span className="font-mono text-xs font-medium">{call.name}</span>
      </div>
      {call.result && (
        <button
          onClick={() => setExpanded(!expanded)}
          className="ml-2 text-xs text-muted-foreground hover:text-foreground"
        >
          {expanded ? "Hide output" : "Show output"}
        </button>
      )}
      {expanded && call.result && (
        <pre className="mt-1.5 p-2 bg-muted/30 rounded-md text-xs overflow-x-auto">{call.result}</pre>
      )}
    </div>
  );
}

// ============================================
// Render helpers
// ============================================

type StyleOption = "current" | "flat" | "minimal" | "claude";

function renderConversation(conversation: ConversationItem[], style: StyleOption) {
  const components = {
    current: { User: CurrentUserMessage, Agent: CurrentAgentMessage, Tool: CurrentToolCall },
    flat: { User: FlatUserMessage, Agent: FlatAgentMessage, Tool: FlatToolCall },
    minimal: { User: MinimalUserMessage, Agent: MinimalAgentMessage, Tool: MinimalToolCall },
    claude: { User: ClaudeUserMessage, Agent: ClaudeAgentMessage, Tool: ClaudeToolCall },
  };

  const { User, Agent, Tool } = components[style];

  return conversation.map((item, i) => {
    switch (item.role) {
      case "user":
        return <User key={i} content={item.content} />;
      case "agent":
        return <Agent key={i} content={item.content} />;
      case "tool":
        return (
          <div key={i} className="space-y-1">
            {item.calls.map((call, j) => (
              <Tool key={j} call={call} />
            ))}
          </div>
        );
    }
  });
}

// ============================================
// Main Page
// ============================================

export default function ChatStylesPage() {
  const [selectedStyle, setSelectedStyle] = useState<StyleOption>("flat");

  if (!isDev) {
    return (
      <div className="flex items-center justify-center min-h-screen">
        <div className="text-center">
          <h1 className="text-4xl font-bold text-muted-foreground">404</h1>
          <p className="text-muted-foreground mt-2">Page not found</p>
        </div>
      </div>
    );
  }

  const styles: { id: StyleOption; name: string; description: string }[] = [
    { id: "current", name: "Current", description: "Existing design with backgrounds for both user and agent" },
    { id: "flat", name: "Flat Agent", description: "User bubble, agent on flat background, compact tools" },
    { id: "minimal", name: "Minimal", description: "Subtle borders, completely flat, inline tools" },
    { id: "claude", name: "Claude-like", description: "Warm accent for user, avatar for agent, boxed tools" },
  ];

  return (
    <div className="min-h-screen bg-background">
      <div className="container mx-auto py-8 px-4 max-w-5xl">
        <div className="mb-6">
          <Link
            href="/dev"
            className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-4"
          >
            <ArrowLeft className="w-4 h-4 mr-2" />
            Back to Developer Tools
          </Link>
          <h1 className="text-2xl font-bold">Chat UI Styles</h1>
          <p className="text-muted-foreground mt-1 text-sm">
            Compare different chat styling approaches
          </p>
          <Badge variant="outline" className="mt-2">Development Mode</Badge>
        </div>

        {/* Style selector */}
        <div className="flex gap-2 mb-6 flex-wrap">
          {styles.map((style) => (
            <button
              key={style.id}
              onClick={() => setSelectedStyle(style.id)}
              className={cn(
                "px-3 py-1.5 rounded-md text-sm transition-colors",
                selectedStyle === style.id
                  ? "bg-primary text-primary-foreground"
                  : "bg-muted hover:bg-muted/80"
              )}
            >
              {style.name}
            </button>
          ))}
        </div>

        {/* Description */}
        <p className="text-sm text-muted-foreground mb-4">
          {styles.find((s) => s.id === selectedStyle)?.description}
        </p>

        {/* Chat preview */}
        <ScrollArea className="h-[calc(100vh-20rem)]">
          <div className="border rounded-lg bg-background">
            {/* Main conversation */}
            <div className="p-4 space-y-4">
              <div className="text-xs text-muted-foreground text-center mb-6">Full Conversation</div>
              {renderConversation(sampleConversation, selectedStyle)}
            </div>

            {/* Executing state */}
            <div className="border-t p-4 space-y-4">
              <div className="text-xs text-muted-foreground text-center mb-4">Executing State</div>
              {renderConversation(executingConversation, selectedStyle)}
            </div>
          </div>
        </ScrollArea>

        {/* Side by side comparison */}
        <div className="mt-8">
          <h2 className="text-lg font-semibold mb-4">Side-by-Side Comparison</h2>
          <div className="grid grid-cols-2 gap-4">
            {styles.map((style) => (
              <div key={style.id} className="border rounded-lg overflow-hidden">
                <div className="bg-muted/30 px-3 py-2 border-b">
                  <span className="font-medium text-sm">{style.name}</span>
                </div>
                <div className="p-3 space-y-3 text-sm">
                  {renderConversation(sampleConversation.slice(0, 4), style.id)}
                </div>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
