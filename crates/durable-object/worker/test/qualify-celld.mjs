// Three real celld nodes, replicated writes and owner SIGKILL. No cloud credentials.
// Requires celld 0.6.2, s3s-fs 0.14.1, the built Wasm application and npm ci.
import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { createServer, request } from "node:http";
import {
  mkdtempSync,
  mkdirSync,
  writeFileSync,
  readFileSync,
  readdirSync,
  existsSync,
  openSync,
  closeSync,
  unlinkSync,
  appendFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, "../../../..");
const run = mkdtempSync(join(tmpdir(), "everruns-celld-qualification."));
const config = join(here, `fleet-${process.pid}.generated.json`);
const native = process.env.CELLD_QUALIFY_NATIVE === "1";
const nativeBootstrap=join(here,`native-fleet-${process.pid}.generated.js`);
const children = [];
const servers = [];
const upgradeSockets = new Set();
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const env = {
  ...process.env,
  AWS_ACCESS_KEY_ID: "qualification",
  AWS_SECRET_ACCESS_KEY: "qualification-secret-for-local-test",
  AWS_EC2_METADATA_DISABLED: "true",
  CELLD_ESBUILD: resolve(here, "../node_modules/esbuild/bin/esbuild"),
};
delete env.AWS_SESSION_TOKEN;
function start(command, args, name, extra = {}) {
  const fd = openSync(join(run, `${name}.log`), "w");
  const child = spawn(command, args, {
    cwd: root,
    env: { ...env, ...extra },
    stdio: ["ignore", fd, fd],
  });
  closeSync(fd);
  child.once("exit", (code, signal) =>
    appendFileSync(join(run, "exits.log"), `${name}: ${code} ${signal}\n`),
  );
  child.failure = null;
  child.on("error", (error) => (child.failure = error));
  children.push(child);
  return child;
}
async function until(label, fn, ms = 90000) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const result = await fn();
    if (result) return result;
    await sleep(100);
  }
  throw new Error(`timeout: ${label}; logs: ${run}`);
}
// freePort releases a port before its owner binds it, so a later port-0 bind can
// be given the same port. Port-0 binds skip every port already handed out.
const reserved = new Set();
async function listen(server, port = 0) {
  servers.push(server);
  for (;;) {
    await new Promise((r, j) => server.once("error", j).listen(port, "127.0.0.1", r));
    const bound = server.address().port;
    if (port !== 0 || !reserved.has(bound)) {
      reserved.add(bound);
      return bound;
    }
    await new Promise((r) => server.close(r));
  }
}
async function freePort() {
  const s = createServer();
  const p = await listen(s);
  await new Promise((r) => s.close(r));
  return p;
}
async function alive(port) {
  try {
    await fetch(`http://127.0.0.1:${port}/`, { signal: AbortSignal.timeout(500) });
    return true;
  } catch {
    return false;
  }
}
async function exit(child) {
  if (child.exitCode !== null || child.signalCode !== null) return;
  await Promise.race([new Promise((r) => child.once("exit", r)), sleep(5000)]);
  if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
}
try {
  assert.match(execFileSync("celld", ["--version"], { encoding: "utf8" }), /0\.6\.2/);
  assert.match(execFileSync("s3s-fs", ["--version"], { encoding: "utf8" }), /0\.14\.1/);
  // A short bucket avoids the filesystem emulator's encoded metadata filename
  // limit. The server implements S3 conditions; serialize operations because its
  // filesystem conditional check and write are otherwise separate operations.
  const storage = join(run, "s3");
  mkdirSync(join(storage, "qaa"), { recursive: true });
  const storagePort = await freePort();
  const s3 = start(
    "s3s-fs",
    [
      "--host",
      "127.0.0.1",
      "--port",
      String(storagePort),
      "--access-key",
      env.AWS_ACCESS_KEY_ID,
      "--secret-key",
      env.AWS_SECRET_ACCESS_KEY,
      storage,
    ],
    "s3",
  );
  await until("S3 ready", async () => {
    if (s3.failure) throw s3.failure;
    return alive(storagePort);
  });
  let tail = Promise.resolve(),
    queued = 0,
    buffered = 0;
  const proxyPort = await listen(
    createServer(async (incoming, response) => {
      if (queued >= 128) {
        response.writeHead(503).end();
        incoming.resume();
        return;
      }
      queued++;
      const chunks = [];
      let size = 0;
      try {
        for await (const chunk of incoming) {
          size += chunk.length;
          buffered += chunk.length;
          if (size > 16 * 1024 * 1024 || buffered > 32 * 1024 * 1024)
            throw new Error("S3 fixture request capacity");
          chunks.push(chunk);
        }
      } catch {
        buffered -= size;
        queued--;
        response.writeHead(413).end();
        return;
      }
      const body = Buffer.concat(chunks);
      tail = tail.then(
        () =>
          new Promise((done) => {
            let settled = false;
            const finish = () => {
              if (!settled) {
                settled = true;
                queued--;
                buffered -= size;
                done();
              }
            };
            const out = request(
              {
                hostname: "127.0.0.1",
                port: storagePort,
                path: incoming.url,
                method: incoming.method,
                headers: incoming.headers,
              },
              (upstream) => {
                // A killed client must not block the serialized S3 queue. Complete its
                // already received operation, drain the reply and then admit the next.
                if (response.destroyed) upstream.resume();
                else {
                  response.writeHead(upstream.statusCode, upstream.headers);
                  upstream.pipe(response);
                  response.once("close", () => {
                    upstream.unpipe(response);
                    upstream.resume();
                  });
                }
                upstream.once("end", finish);
                upstream.once("error", finish);
              },
            );
            out.setTimeout(15000, () => out.destroy(new Error("S3 request timeout")));
            out.once("error", () => {
              if (!response.headersSent) response.writeHead(502);
              response.end();
              finish();
            });
            out.end(body);
          }),
      );
    }),
  );
  const fleet = [
    "--bucket",
    "s3://qaa",
    "--endpoint",
    `http://127.0.0.1:${proxyPort}`,
    "--region",
    "us-east-1",
  ];
  const fixture = JSON.parse(readFileSync(join(here, "qualification.json"), "utf8"));
  fixture.vars.AUTHORITY_TOKEN = "authority-test-token-at-least-32-bytes";
  const gatewayPort=await freePort();
  fixture.vars.NOTIFICATION_ORIGIN=`http://127.0.0.1:${gatewayPort}`;
  const nativePort=native?await freePort():null;
  if(native) {
    writeFileSync(nativeBootstrap,`export {default,SessionAuthority,ServeExecutor} from ${JSON.stringify(resolve(root,"examples/serve/celld/worker/src/index.js"))};\n`);
    fixture.main=`native-fleet-${process.pid}.generated.js`;
    fixture.vars={TENANT_ID:"test",SERVE_URL:`http://127.0.0.1:${nativePort}`,AUTHORITY_TOKEN:"synthetic-authority-credential-at-least-32-bytes",EXECUTOR_TOKEN:"synthetic-executor-credential-at-least-32-bytes",API_TOKEN:"synthetic-api-credential-at-least-32-bytes"};
  }
  writeFileSync(config, JSON.stringify(fixture));
  const deploy = start("celld", ["deploy", config, ...fleet, "--json"], "deploy");
  await exit(deploy);
  assert.equal(deploy.exitCode, 0, `deploy failed: ${run}`);
  const nodes = [];
  for (let i = 0; i < 3; i++) {
    const port = await freePort(),
      peer = await freePort(),
      name = `qualification-${i}`;
    const child = start(
      "celld",
      [
        ...fleet,
        "--listen",
        `127.0.0.1:${port}`,
        "--internal-listen",
        `127.0.0.1:${peer}`,
        "--advertise",
        `127.0.0.1:${peer}`,
      ],
      name,
      {
        RUST_LOG: "info",
        CELLD_NODE: name,
        CELLD_DURABILITY: "fleet",
        CELLD_WATCH: join(run, name),
        CELLD_TOKIO_THREADS: "2",
        CELLD_MAX_RSS_MB: "0",
        CELLD_TTL_MS: "10000",
        CELLD_WAKER_TICK_MS: "1000",
        CELLD_READY_FLEET_GATE_MS: "10000",
      },
    );
    nodes.push({ name, port, child });
  }
  for (const node of nodes) await until(`${node.name} ready`, () => alive(node.port));
  // No POST retry: a lost acknowledgement must be retried with the same identity
  // by the caller. After the kill, route new reads only to surviving nodes.
  let live = [...nodes];
  const gateway = await listen(
    createServer((req, res) => {
      const out = request(
        {
          hostname: "127.0.0.1",
          port: live[0].port,
          method: req.method,
          path: req.url,
          headers: req.headers,
        },
        (reply) => {
          res.writeHead(reply.statusCode, reply.headers);
          reply.pipe(res);
        },
      );
      out.once("error", (error) => {
        appendFileSync(
          join(run, "gateway.log"),
          `${error.stack} ${JSON.stringify(live.map((n) => ({ name: n.name, port: n.port, exit: n.child.exitCode, signal: n.child.signalCode })))}\n`,
        );
        res.writeHead(502).end();
      });
      req.pipe(out);
    }).on("upgrade",(req,client,head)=>{
      upgradeSockets.add(client);
      let peer;
      const out=request({hostname:"127.0.0.1",port:live[0].port,path:req.url,method:req.method,headers:req.headers});
      const close=()=>{client.destroy();peer?.destroy();out.destroy();};
      client.on("error",close);
      client.on("close",()=>{upgradeSockets.delete(client);peer?.destroy();});
      out.on("error",close);
      out.on("response",reply=>{reply.resume();client.end(`HTTP/1.1 ${reply.statusCode} ${reply.statusMessage}\r\nConnection: close\r\n\r\n`);});
      out.on("upgrade",(reply,socket,replyHead)=>{
        peer=socket;upgradeSockets.add(peer);
        peer.on("error",close);
        peer.on("close",()=>{upgradeSockets.delete(peer);client.destroy();});
        let headers=`HTTP/1.1 ${reply.statusCode} ${reply.statusMessage}\r\n`;
        for(let i=0;i<reply.rawHeaders.length;i+=2)headers+=`${reply.rawHeaders[i]}: ${reply.rawHeaders[i+1]}\r\n`;
        client.write(`${headers}\r\n`);
        if(head.length)peer.write(head);
        if(replyHead.length)client.write(replyHead);
        client.pipe(peer).pipe(client);
      });
      out.end();
    }),gatewayPort,
  );
  const signal = join(run, "effect.json");
  const check = start(process.execPath, [native?resolve(root,"examples/serve/celld/worker/test/e2e.mjs"):join(here, "qualify-isolate.mjs")], "proof", {
    DO_QUALIFY_URL: `http://127.0.0.1:${gateway}`,
    DO_QUALIFY_BACKEND: "celld-0.6.2-three-node",
    DO_QUALIFY_SIGNAL_PATH: signal,
    DO_QUALIFY_NO_CLIENT_MS: "45000",
    ...(native?{SERVE_TEST_AUTHORITY_URL:`http://127.0.0.1:${gateway}`,SERVE_TEST_NATIVE_PORT:String(nativePort),SERVE_TEST_OWNER_LOSS_SIGNAL:signal,SERVE_TEST_NO_CLIENT_MS:"45000",SERVE_TEST_KILL_EXECUTOR:"false",SERVE_TEST_TOOL_DELAY_MS:"10000"}:{}),
  });
  await until("tool effect", () => {
    if (check.exitCode !== null) throw new Error(readFileSync(join(run, "proof.log"), "utf8"));
    return existsSync(signal);
  });
  const cells = readdirSync(join(storage, "qaa/cells"));
  assert.equal(cells.length, 1);
  const owner = JSON.parse(readFileSync(join(storage, "qaa/cells", cells[0], "own.json"), "utf8"));
  const killed = nodes.find((n) => n.name === owner.node);
  assert.ok(killed, "the effect owner belongs to this test fleet");
  killed.child.kill("SIGKILL");
  await exit(killed.child);
  live = nodes.filter((n) => n !== killed);
  writeFileSync(`${signal}.continue`, "");
  await until(
    "autonomous recovery proof",
    () => check.exitCode !== null || check.signalCode !== null,
    120000,
  );
  assert.equal(check.exitCode, 0, readFileSync(join(run, "proof.log"), "utf8"));
  if (!native) {
    // After autonomous recovery has already passed, qualify the passive SSE
    // bridge against celld's actual hibernating WebSocket implementation.
    const {session}=JSON.parse(readFileSync(signal,"utf8"));
    const hash=await crypto.subtle.digest("SHA-256",new TextEncoder().encode(`${fixture.vars.AUTHORITY_TOKEN}:api`));
    const apiToken=[...new Uint8Array(hash)].map(byte=>byte.toString(16).padStart(2,"0")).join("");
    // Every survivor must serve observers, including a frontend on a different
    // node from the recovered object. A single random ingress can hide that hop.
    const observers=[];
    try {
      for (const node of live) {
        const observer=await fetch(`http://127.0.0.1:${node.port}/v1/sessions/${session}/sse`,{
          headers:{authorization:`Bearer ${apiToken}`},signal:AbortSignal.timeout(20000),
        });
        assert.equal(observer.status,200,`${node.name}: celld supports the passive SSE bridge${observer.ok?"":`: ${await observer.text()}`}`);
        const reader=observer.body.getReader();
        observers.push({node,reader});
        assert.match(new TextDecoder().decode((await reader.read()).value),/event: connected/);
      }
      const marker=`observer-probe-${crypto.randomUUID()}`;
      // Let initial canonical replay drain: this input must arrive through
      // a later notification rather than racing the initial snapshot.
      await sleep(1500);
      const deliveries=observers.map(async({node,reader})=>{
        let text="";const decoder=new TextDecoder();
        while(!text.includes(marker)) {
          const next=await reader.read();
          assert.ok(!next.done,`${node.name}: observer closed before receiving the new input`);
          text+=decoder.decode(next.value,{stream:true});
        }
      });
      const admission=fetch(`http://127.0.0.1:${gateway}/v1/sessions/${session}/messages`,{
        method:"POST",headers:{authorization:`Bearer ${apiToken}`,"content-type":"application/json"},
        body:JSON.stringify({message:{role:"user",content:[{type:"text",text:marker}]}}),
      });
      await Promise.all([...deliveries,admission.then(reply=>assert.equal(reply.status,201))]);
      console.log(JSON.stringify({passive_observers:observers.length,new_input_delivered:true}));
    } finally {
      await Promise.allSettled(observers.map(({reader})=>reader.cancel()));
    }
  }
  console.log(readFileSync(join(run, "proof.log"), "utf8").trim());
  console.log(`Celld qualification evidence: ${run}`);
} finally {
  for(const socket of upgradeSockets)socket.destroy();
  // Stop dependants before the test storage service.
  for (const child of children.slice(1))
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGTERM");
  await Promise.all(children.slice(1).map(exit));
  const storage = children[0];
  if (storage?.exitCode === null && storage.signalCode === null) storage.kill("SIGTERM");
  if (storage) await exit(storage);
  for (const server of servers) {
    server.closeAllConnections();
    server.close();
  }
  if (existsSync(config)) unlinkSync(config);
  if (existsSync(nativeBootstrap)) unlinkSync(nativeBootstrap);
}
