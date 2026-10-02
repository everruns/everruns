# celld (serve, experimental)

A [serve](../../../crates/serve) app that runs durably on
[celld](https://github.com/denoland/celld) with
[serve-celld](../../../crates/serve-celld). The app is ordinary serve code; only
`main` changes, from `serve::start` to `serve_celld::start`. `worker/` is the
celld application: a Durable Object per name that runs the app in a container and
keeps its state. The full guide is
[Serve on celld](https://docs.everruns.com/framework/serve-celld/).

## Try it without containers

The external cell reaches a serve process by URL instead of a container, so this
needs only [celld](https://celld.dev) and Node.js:

```sh
cargo run -p serve-example-celld -- celld --dev --port 8080   # the container half, offline
cd examples/serve/celld/worker && npm install
celld dev wrangler.external.jsonc                             # the cells, on :9876
```

```sh
S=$(curl -s -XPOST localhost:9876/cells/demo/v1/sessions -H 'content-type: application/json' -d '{}' | jq -r .id)
curl -s -XPOST localhost:9876/cells/demo/v1/sessions/$S/messages \
  -H 'content-type: application/json' \
  -d '{"message":{"role":"user","content":[{"type":"text","text":"look up celld"}]}}'
curl -s localhost:9876/cells/demo/_cell/state
# {"boot_id":"…","snapshot_seq":1,"snapshot_bytes":…,"journal":1,"restores":0,"replayed":0}
```

Kill the serve process while the turn runs (the tool sleeps for
`RESEARCH_DELAY_MS`, 2 s by default), start it again, and read the session: the
cell restores the last snapshot and replays the message.

`npm run test:e2e` does exactly that against a real `celld dev` node, then kills
the node itself with a turn outstanding and checks that the cell's alarm snapshots
it after the restart. Build the binary first with
`cargo build -p serve-example-celld`.

## Run it with containers

Every celld node that serves the cell needs Docker or Podman.

```sh
docker build -f examples/serve/celld/Dockerfile -t serve-celld-example .   # repository root
cd examples/serve/celld/worker && npm install && celld dev
```

`celld deploy` builds or pulls images for `linux/amd64`, so for a fleet push the
image to a registry the deploying machine can pull and name it in
`wrangler.jsonc`. Pass the model gateway as variables: names starting with
`SERVE_` (`SERVE_GATEWAY_URL`, `SERVE_GATEWAY_KEY`) reach the container, and
`CONTAINER_ENV` lists any others.

| File | Is |
|---|---|
| `src/main.rs` | `serve_celld::start(...)` instead of `serve::start(...)` |
| `src/agent.rs` | `#[agent] fn assistant()`, with an offline script |
| `src/tools.rs` | `#[tool] async fn look_up(cx, topic)`, slow on purpose |
| `Dockerfile` | the container image |
| `worker/src/cell.js` | the cell: snapshot, journal, restore and replay |
| `worker/src/index.js` | the Worker and the two cell classes (container, external) |
| `worker/wrangler.jsonc` | the container deployment |
| `worker/wrangler.external.jsonc` | the same cells over `SERVE_URL` |
| `worker/test/e2e.mjs` | crash and node-loss recovery on a real celld node |
