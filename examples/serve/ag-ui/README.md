# ag-ui (serve, experimental)

A [serve](../../../crates/serve) app streamed to CopilotKit or any
[AG-UI](https://docs.ag-ui.com) 1.0 client. The `ag-ui` feature serves
`POST /v1/channels/assistant/ag-ui`, the same route shape as an Everruns endpoint, so
a front end moves between the two by base URL alone. The `deploy` tool needs
approval, which the client sees as an interrupt. It runs offline; without a
model gateway, the agent follows a scripted simulator.

```sh
cargo run -p serve-example-ag-ui              # dev server on :3000
cargo run -p serve-example-ag-ui -- eval      # 1 passed
```

## Drive it with `@ag-ui/client`

`client/run.mjs` is an `HttpAgent`, the client CopilotKit builds on. Its first
run ends with the `tool_approval` interrupt; the second sends a `resume` entry
and streams the rest of the same turn:

```sh
cd examples/serve/ag-ui/client
npm install
npm start               # approves; `npm start -- reject` rejects
```

```text
[RUN_STARTED]
[RUN_FINISHED]
interrupt call_llmsim_0_0: tool_approval -> allow
[RUN_STARTED]
Deployed to staging. (Offline demo reply: the deployment is in the tool result.)
[TEXT_MESSAGE_END]
[RUN_FINISHED]
```

The offline script replies the same way either way; on a real model a
rejected call never runs and the reply says so. `SERVE_URL` overrides the base
URL.

## Plug it into CopilotKit

Register the route as an agent in CopilotKit's runtime, for example in a
Next.js route handler:

```ts
import { HttpAgent } from "@ag-ui/client";
import { CopilotRuntime } from "@copilotkit/runtime";

const runtime = new CopilotRuntime({
  agents: {
    assistant: new HttpAgent({ url: "http://127.0.0.1:3000/v1/channels/assistant/ag-ui" }),
  },
});
```

| File | Is |
|---|---|
| `agent/instructions.md` | the always-on prompt (hot-reloads in `dev`) |
| `src/agent.rs` | `#[agent] fn assistant()` |
| `src/tools/deploy.rs` | `#[tool(needs_approval)] async fn deploy(cx, environment)` |
| `evals/deploy.rs` | `#[eval] async fn deploys_after_approval(t)` |
| `client/run.mjs` | the `@ag-ui/client` driver |

Each AG-UI `threadId` maps to one session, which survives a restart, along
with a turn parked on an approval or a question: after a restart the interrupt
is still open. Pending approvals and questions can also be answered through `/v1/sessions/{id}/approvals/{tool_call_id}` and
`/question-answers`. Set `OPENROUTER_API_KEY` to run the agent on a real model.
