# DeepSec

This directory holds the [DeepSec](https://www.npmjs.com/package/deepsec)
workspace for Everruns (target: `..`). The config, threat model, and generated
matchers are checked in; scan state and credentials stay local.

## Setup

The configured agent is Codex with `gpt-5.6-sol` at `xhigh` reasoning. The model
route uses the machine's Codex login, so check `codex login status` first.

```bash
cd .deepsec
pnpm install
pnpm deepsec setup --project-id everruns --agent codex --model-auth local --yes
```

Setup inventories HTTP, RPC, queue, cron, CLI, webhook, and agent-tool entry
points; checks matcher coverage; generates scoped matchers when needed; and
starts the AI investigation. It saves checkpoints under `data/everruns/setup/`
and resumes from the first incomplete phase. Review changes to
`generated-matchers.ts` and `data/everruns/INFO.md` before committing them.

For another credential route, follow
`node_modules/deepsec/dist/docs/vercel-setup.md`. Keep keys in the process
environment or ignored `.env.local`, never in this config.

## Subsequent scans

```bash
pnpm deepsec setup --project-id everruns --status --output json
pnpm deepsec scan --project-id everruns
pnpm deepsec process --project-id everruns --concurrency 5
pnpm deepsec revalidate --project-id everruns --concurrency 5
pnpm deepsec report --project-id everruns
pnpm deepsec export --project-id everruns --format md-dir --out ./findings
```

`scan` is local pattern matching. `process` uses Codex and resumes pending files
after interruption. The project currently prioritizes UI, server, core,
worker, host, and integrations; priority changes processing order, not the
scan's repository scope.

## Adding another project

```bash
pnpm deepsec init-project ../some-other-package
pnpm deepsec setup --project-id some-other-package
```

## Reference

- `node_modules/deepsec/SKILL.md` — bundled agent instructions
- `node_modules/deepsec/dist/docs/` — commands, configuration, models,
  matchers, and data layout for the installed version
