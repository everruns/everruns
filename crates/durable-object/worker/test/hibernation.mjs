// Actual workerd: pending timers block eviction, durable retries do not, and
// an SSE observer retains a hibernating socket across the alarm's new instance.
import assert from "node:assert/strict";
import { build } from "esbuild";
import { Miniflare, convertV4MiniflareOptions } from "miniflare";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

const here = dirname(fileURLToPath(import.meta.url));
const run = await mkdtemp(join(tmpdir(), "everruns-hibernation."));
const bundle = await build({ entryPoints: [join(here, "hibernation-fixture.js")], bundle: true, write: false,
  format: "esm", platform: "browser", external: ["cloudflare:workers", "node:crypto"],
  plugins: [{ name: "engine-wasm", setup(build) { build.onResolve({filter:/\.wasm$/}, () => ({path:"./engine.wasm",external:true})); } }],
});
const AUTHORITY_TOKEN = "hibernation-private-authority-token-32-bytes";
const API_TOKEN = "hibernation-public-api-token-32-bytes";
const mf = new Miniflare(convertV4MiniflareOptions({
  name: "hibernation", compatibilityDate: "2026-07-01", compatibilityFlags: ["nodejs_compat"],
  modules: [
    {type:"ESModule",path:join(run,"worker.js"),contents:bundle.outputFiles[0].text},
    {type:"CompiledWasm",path:join(run,"engine.wasm"),contents:await readFile(resolve(here,"../../../../examples/durable-object-agent/pkg/everruns_object_agent_bg.wasm"))},
  ], modulesRoot: run,
  durableObjects: {SESSIONS:{className:"SessionAuthority",useSQLite:true}},
  durableObjectsPersist: join(run,"state"),
  bindings: {AUTHORITY_TOKEN,API_TOKEN,TENANT_ID:"test",MODEL:"gpt-6-luna"},
}));
const id = `session_${crypto.randomUUID().replaceAll("-", "")}`;
const address = `https://test/tenants/test/sessions/${id}/`;
const internal = async op => {
  const response = await mf.dispatchFetch(address+op,{method:"POST",headers:{authorization:`Bearer ${AUTHORITY_TOKEN}`,"content-type":"application/json"},body:"{}"});
  assert.equal(response.status,200,await response.clone().text());return response.json();
};
const api = async (path,body) => {
  const response = await mf.dispatchFetch(`https://test/v1/sessions${path}`,{method:body?"POST":"GET",headers:{authorization:`Bearer ${API_TOKEN}`,"content-type":"application/json"},...(body?{body:JSON.stringify(body)}:{})});
  if(!response.ok)assert.fail(await response.text());return response;
};
let reader;
try {
  await (await api("",{id})).json();
  await internal("hold");
  const eviction = mf.unsafeEvictDurableObject("hibernation","SessionAuthority",{name:`test:${id}`,webSockets:"hibernate"})
    .then(()=>"evicted",error=>{assert.match(error.message,/active references/);return "blocked";});
  assert.equal(await Promise.race([eviction,delay(500,"blocked")]),"blocked","negative control: a pending timer must block eviction");
  await internal("clear");
  await eviction;
  console.log("PASS: pending timer blocks eviction");

  const stream = await api(`/${id}/sse`);
  reader=stream.body.getReader();await reader.read();
  let observed="";
  const observing=(async()=>{while(!observed.includes("event: turn.completed")){const result=await reader.read();assert.ok(!result.done,"SSE survives hibernation");observed+=new TextDecoder().decode(result.value);}})();
  const message=`message_${crypto.randomUUID().replaceAll("-", "")}`;
  await (await api(`/${id}/messages`,{message_id:message,message:{role:"user",content:[{type:"text",text:"hello"}]}})).json();
  let parked;
  for(let attempt=0;attempt<100;attempt++) {
    const probe=await internal("probe");
    if(probe.state.wake_at && !probe.state.holder){parked=probe;break;}
    await delay(50);
  }
  assert.ok(parked,"Engine must park with a retained wake");
  console.log("PASS: Engine released its epoch for provider backoff");
  assert.equal(parked.state.runnable,false);
  await Promise.race([
    mf.unsafeEvictDurableObject("hibernation","SessionAuthority",{name:`test:${id}`,webSockets:"hibernate"}),
    delay(2000).then(()=>{throw new Error("parked Engine still prevents eviction");}),
  ]);
  // No requests to the object or client wake during this interval. Only its
  // durable alarm may resume it; the open observer must remain passive.
  await Promise.race([observing,delay(18000).then(()=>{throw new Error("alarm did not resume the original admission");})]);
  // turn.completed precedes Framework finalization/release. After the alarm's
  // autonomous progress is visible, read-only probes may await that boundary.
  let resumed;
  for(let attempt=0;attempt<100;attempt++) {
    resumed=await internal("probe");
    if(!resumed.state.holder)break;
    await delay(20);
  }
  assert.notEqual(resumed.instance,parked.instance,"the alarm opened a new object instance");
  assert.equal(resumed.state.holder,null);
  const outcome=await (await api(`/${id}/admissions/${message}`)).json();
  assert.equal(outcome.status,"complete");assert.equal(outcome.turn.success,true);
  console.log("PASS: real workerd parks, hibernates with SSE connected, and resumes by alarm");
} finally {
  // Dispose the runtime concurrently: proxy stream cancellation can wait for
  // its open WebSocket's close handshake until the runtime is stopped.
  const cancelled=reader?.cancel().catch(()=>{});
  await mf.dispose();
  await cancelled;
  await rm(run,{recursive:true,force:true});
}
