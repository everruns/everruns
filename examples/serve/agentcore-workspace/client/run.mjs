// Drives the agentcore-workspace example with `@ag-ui/client`, locally or on
// AgentCore Runtime: one run that ends on the `share_report` approval
// interrupt, then a resuming run that answers it.
//
//   cargo run -p serve-example-agentcore-workspace -- agentcore --dev
//   npm install && npm start                 # or: npm start -- reject
//
// On AgentCore (AG-UI protocol, JWT inbound auth):
//   AGENTCORE_URL="https://bedrock-agentcore.$REGION.amazonaws.com/runtimes/$(node -p 'encodeURIComponent(process.argv[1])' "$AGENT_ARN")/invocations?qualifier=DEFAULT" \
//   AGENTCORE_TOKEN="$BEARER_TOKEN" npm start
import { randomUUID } from "node:crypto";
import { HttpAgent } from "@ag-ui/client";

const url = process.env.AGENTCORE_URL ?? "http://127.0.0.1:8080/invocations";
const decision = process.argv[2] === "reject" ? "reject" : "allow";
// AgentCore routes by this header; one id is one microVM and one workspace.
// Reuse an id (AGENTCORE_SESSION) to come back to the same workspace.
const session = process.env.AGENTCORE_SESSION ?? `${randomUUID()}-${randomUUID()}`.slice(0, 48);

const headers = { "X-Amzn-Bedrock-AgentCore-Runtime-Session-Id": session };
if (process.env.AGENTCORE_TOKEN) headers.Authorization = `Bearer ${process.env.AGENTCORE_TOKEN}`;

const agent = new HttpAgent({ url, headers, threadId: session });

// Print the stream as it arrives; return the open interrupts, if any.
async function run(params = {}) {
  let interrupts = [];
  await agent.runAgent(params, {
    onEvent({ event }) {
      if (event.type === "TEXT_MESSAGE_CONTENT") {
        process.stdout.write(event.delta);
      } else if (event.type !== "TEXT_MESSAGE_START") {
        if (event.type === "TEXT_MESSAGE_END") process.stdout.write("\n");
        console.log(`[${event.type}]`);
      }
    },
    onRunFinishedEvent({ event }) {
      if (event.outcome?.type === "interrupt") interrupts = event.outcome.interrupts;
    },
  });
  return interrupts;
}

console.log(`session ${session}`);
agent.addMessage({
  id: randomUUID(),
  role: "user",
  content: "Add 'check the build' to my todo list and share it.",
});
const open = await run();
for (const interrupt of open) {
  console.log(`interrupt ${interrupt.id}: ${interrupt.reason} -> ${decision}`);
}
if (open.length > 0) {
  await run({
    resume: open.map((interrupt) => ({
      interruptId: interrupt.id,
      status: "resolved",
      payload: { decision },
    })),
  });
}
