# Choose an execution option

Framework and Server are both platforms. Select execution options on either;
agentd runs them on executors. Do not present Monty as a Framework/Server
replacement or as replacing Bashkit.

- Prefer native tools for individual typed operations, images or user interaction.
- Choose Bashkit when shell scripts, pipelines or the established shell code-mode
  interface fit. Bashkit already runs Python (CPython WASM in Everruns).
- Choose `python_code_mode` when Python is the preferred programmatic tool
  interface and saved-program approval continuations matter. It uses Monty,
  with a separate pinned subprocess runtime and private checkpoint/effect journal.
- Choose a full session sandbox for native packages, processes and OS workspaces.

Do not justify Monty merely by saying it performs Python calculations. Do not
claim it supports arbitrary CPython packages or is inherently faster. Multiple
options can be attached explicitly; set a preferred interface and do not rerun
failed source in another runtime automatically.

## Python Code Mode

Enable Cargo feature `monty`; Framework agents attach `PythonCodeMode` from
`everruns`, while Server agents attach capability id `python_code_mode`.
Both require runtime provisioning on the execution host. Read the current
[capability docs](https://everruns.com/docs/capabilities/python-code-mode/)
for the pinned version, Linux kernel requirements, limits and private storage.
The default hardened launcher fails closed without its kernel protections.

The `python` tool takes either `script` or `resume`. `tools(query)` discovers
eligible current schemas; `tool(name, arguments)` returns `{result, error}`
after normal authorization and output policy. Files and networking use these
existing tools. Never ask the agent to put credentials or snapshots in source,
workspace files or session KV. Native and direct-only tools remain available;
this capability does not compress the native tool list.

When an approval pauses Python, preserve `python_resume`. After the decision,
call `python` with `{"resume": token}`. Never repeat the source or execute the
pending effect separately. An unsettled effect is uncertain and must not be
retried automatically. A persistent Python journal does not provide session
recovery unless the Framework hosting profile is also durable.

Bashkit's optional upstream Monty feature is separate. Everruns' `monty` feature
does not enable it, change Bashkit's Python executable or install a process-wide
Monty allocator.

See [selection guide](https://everruns.com/docs/framework/execution-options/)
and [Framework example](https://everruns.com/docs/framework/python-code-mode/).

Use the repository's `evals/shell-code-mode` and `evals/mcp-scale` studies to
compare interfaces with the same tasks, model and repetitions. Require actual
execution through the selected surface; direct-tool fallback is not Monty
success. Keep model-quality results separate from runtime isolation and hosted
Server/agentd qualification, and report missing or unsupported infrastructure.
