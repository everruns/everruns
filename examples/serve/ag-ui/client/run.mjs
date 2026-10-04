// Drives the serve ag-ui example with `@ag-ui/client`, the client CopilotKit
// builds on: one run that ends on the `deploy` approval interrupt, then a
// resuming run that answers it and streams the rest of the turn.
//
//   cargo run -p serve-example-ag-ui            # in another terminal
//   npm install && npm start                    # or: npm start -- reject
//
// SERVE_URL overrides the base URL; the same code points at an Everruns
// endpoint (`https://<host>/v1/channels/<channel_id>/ag-ui`) by URL alone.
import { randomUUID } from "node:crypto";
import { HttpAgent } from "@ag-ui/client";

const base = process.env.SERVE_URL ?? "http://127.0.0.1:3000";
const decision = process.argv[2] === "reject" ? "reject" : "allow";

const agent = new HttpAgent({
  url: `${base}/v1/channels/assistant/ag-ui`,
  threadId: randomUUID(),
});

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
      if (event.usage) console.log("usage:", JSON.stringify(event.usage));
    },
  });
  return interrupts;
}

agent.addMessage({ id: randomUUID(), role: "user", content: "Deploy to staging, please." });
const open = await run();
if (open.length === 0) {
  console.error("expected the deploy approval interrupt");
  process.exit(1);
}
for (const interrupt of open) {
  console.log(`interrupt ${interrupt.id}: ${interrupt.reason} -> ${decision}`);
}

// A resuming run sends no new message: one entry per open interrupt.
await run({
  resume: open.map((interrupt) => ({
    interruptId: interrupt.id,
    status: "resolved",
    payload: { decision },
  })),
});
