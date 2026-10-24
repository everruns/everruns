// Drives Everruns' /mcp Tasks extension (2026-07-28) with the official MCP
// TypeScript client: `@modelcontextprotocol/client` connects and negotiates the
// 2026-07-28 protocol, and `@modelcontextprotocol/ext-tasks` runs each tool
// call as a task. ext-tasks validates every CreateTaskResult and tasks/*
// response against its published schemas, so a shape we get wrong fails here.
//
// Run by the domain test `official_ts_client_drives_mcp_tasks`
// (scripts/mcp-tasks-interop.sh), whose stand-in runner answers turn 1 and 2
// with "Reply 1" and "Reply 2", and parks turn 3 on an ask_user question set.
//
// Prints `PASS <check>` per check and `OK` at the end; any failure throws.

import { Client, StreamableHTTPClientTransport } from "@modelcontextprotocol/client";
import {
  createTaskSessionFromClient,
  resultFromTaskOutcome,
} from "@modelcontextprotocol/ext-tasks/client";

const PROTOCOL = "2026-07-28";
const server = process.env.MCP_SERVER;
if (!server) throw new Error("MCP_SERVER is not set");
const endpoint = `${server}/mcp`;
const clientInfo = { name: "everruns-tasks-interop", version: "1.0.0" };
const clientCapabilities = { elicitation: { form: {} } };

function check(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(`FAIL ${message}`);
}

function describe(outcome: any): string {
  const error = outcome?.error;
  return error ? `${error.message} ${error.cause ?? ""}` : JSON.stringify(outcome);
}

function text(result: any): string {
  return result?.content?.[0]?.text ?? "";
}

// Every request ext-tasks sends for a task goes through here, so the test also
// sees which headers it attaches (the HTTP binding requires `Mcp-Name`).
const seenTaskHeaders: Record<string, string>[] = [];
async function rawDispatch(request: any, options?: any) {
  const headers: Record<string, string> = {
    "content-type": "application/json",
    accept: "application/json, text/event-stream",
    "mcp-protocol-version": PROTOCOL,
    "mcp-method": request.method,
    ...(options?.context?.headers ?? {}),
  };
  if (String(request.method).startsWith("tasks/")) seenTaskHeaders.push(headers);
  const response = await fetch(endpoint, {
    method: "POST",
    headers,
    body: JSON.stringify({ jsonrpc: "2.0", id: crypto.randomUUID(), ...request }),
    signal: options?.signal,
  });
  const body: any = await response.json();
  return body.error
    ? { kind: "error" as const, error: body.error }
    : { kind: "result" as const, result: body.result };
}

const client = new Client(clientInfo, {
  capabilities: clientCapabilities,
  versionNegotiation: { mode: { pin: PROTOCOL } },
});
await client.connect(new StreamableHTTPClientTransport(new URL(endpoint)));
check(client.getProtocolEra() === "modern", "client negotiated the 2026-07-28 era");

let asked: any;
const session = createTaskSessionFromClient(client, {
  endpointId: "everruns-tasks-interop",
  rawDispatch,
  v2RequestFraming: { protocolVersion: PROTOCOL, clientInfo, clientCapabilities },
  onInputRequest: async (request: any) => {
    asked = request;
    return { action: "accept", content: { target: "Production", areas__0: true, areas__1: false } };
  },
});
check(session.capabilities.execution, "server advertises the Tasks extension");
console.log("PASS discovery");

// 1. A tool call becomes a task and settles with the turn's reply.
const first = await session.callTool("agent_run", { message: "interop one" });
check(first.kind === "task", `agent_run ran as a task (got ${first.kind})`);
const firstTaskId = first.handle.taskId;
const firstDone = await first.settle();
check(
  firstDone.outcome.status === "completed",
  `first task completed (${firstDone.outcome.status}: ${describe(firstDone.outcome)})`,
);
const firstResult = resultFromTaskOutcome(firstDone.outcome);
check(text(firstResult) === "Reply 1", `first reply is "Reply 1" (got ${JSON.stringify(firstResult)})`);
check(firstDone.lastTask?.createdAt && firstDone.lastTask?.lastUpdatedAt, "task carries its timestamps");
console.log("PASS agent_run task settles with its reply");

// 2. The next message is its own task; the first one stays as it was.
const sessionId = (firstResult as any).structuredContent?.session_id;
check(typeof sessionId === "string", "result names the session");
console.log(`SESSION ${sessionId}`);
const second = await session.callTool("session_send_message", {
  session_id: sessionId,
  message: "interop two",
});
check(second.kind === "task", "session_send_message ran as a task");
check(second.handle.taskId !== firstTaskId, "the second message is a new task");
const secondResult = resultFromTaskOutcome((await second.settle()).outcome);
check(text(secondResult) === "Reply 2", `second reply is "Reply 2" (got ${text(secondResult)})`);
const firstAgain = session.task(firstTaskId);
const snapshot = await firstAgain.snapshot();
check(snapshot.status === "completed", "a finished task stays completed");
const firstOutcome = await firstAgain.result();
check(text(resultFromTaskOutcome(firstOutcome)) === "Reply 1", "a finished task keeps its own reply");
console.log("PASS each message is its own task and finished tasks never change");

// 3. A parked question reaches the client as input_required, and the answer
//    (tasks/update) resumes the turn.
const third = await session.callTool("session_send_message", {
  session_id: sessionId,
  message: "interop three",
});
check(third.kind === "task", "the asking turn ran as a task");
const thirdResult = resultFromTaskOutcome((await third.settle()).outcome);
check(asked?.kind === "elicitation", `the question arrived as an elicitation (${JSON.stringify(asked)})`);
check(
  asked.params?.requestedSchema?.properties?.target?.enum?.includes("Production"),
  "the form offers the question's options",
);
check(text(thirdResult) === "Answered", `the resumed turn finished (got ${text(thirdResult)})`);
console.log("PASS input_required round trip through tasks/update");

// 4. Cancel is acknowledged, also on a finished task.
await session.task(firstTaskId).cancel();
console.log("PASS tasks/cancel acknowledged");

check(seenTaskHeaders.length > 0, "tasks/* requests were sent");
check(
  seenTaskHeaders.every((headers) => Object.entries(headers).some(([k, v]) => k.toLowerCase() === "mcp-name" && v.length > 0)),
  "every tasks/* request carried Mcp-Name",
);
console.log("PASS Mcp-Name on tasks/*");

await session.close();
await client.close();
console.log("OK");
