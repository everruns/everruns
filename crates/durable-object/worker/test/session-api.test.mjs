import test from "node:test";
import assert from "node:assert/strict";
import { sessionApi } from "../src/session-api.js";

const API_TOKEN="public-api-test-token-at-least-32-bytes";
const AUTHORITY_TOKEN="private-authority-test-token-at-least-32-bytes";
const session=`session_${"1".repeat(32)}`;
const message=`message_${"2".repeat(32)}`;
class WatchSocket extends EventTarget {
  accept() {}
  close() { this.dispatchEvent(new Event("close")); }
  change() { this.dispatchEvent(new Event("message")); }
}
function fixture() {
  const calls=[];
  const env={TENANT_ID:"test",API_TOKEN,AUTHORITY_TOKEN,SESSIONS:{
    idFromName: name=>name,
    get: name=>({fetch:async request=>{
      calls.push({name,path:new URL(request.url).pathname,authorization:request.headers.get("authorization"),...await request.json()});
      return Response.json({accepted:true});
    }}),
  }};
  return {env,calls};
}
function request(path,body,token=API_TOKEN) {
  return new Request(`https://example.test${path}`,{method:body===undefined?"GET":"POST",headers:{authorization:`Bearer ${token}`,"content-type":"application/json"},...(body===undefined?{}:{body:JSON.stringify(body)})});
}
test("Workers session creation and admission route through Engine at stable object identities",async()=>{
  const {env,calls}=fixture();
  assert.equal((await sessionApi(request("/v1/sessions",{id:session}),env)).status,201);
  assert.equal((await sessionApi(request(`/v1/sessions/${session}/messages`,{message:{id:message,role:"user",content:[{type:"text",text:"hello"}]}}),env)).status,201);
  assert.deepEqual(calls.map(call=>call.operation),["create","send"]);
  assert.ok(calls.every(call=>call.name===`test:${session}`&&call.path.endsWith("/engine")));
  assert.equal(calls[1].body.message_id,message);
  assert.equal(calls[1].authorization,`Bearer ${AUTHORITY_TOKEN}`);
});
test("API credentials cannot select internal authority operations or another object address",async()=>{
  const {env,calls}=fixture();
  assert.equal((await sessionApi(request(`/v1/sessions/${session}`,undefined,"wrong"),env)).status,401);
  assert.equal((await sessionApi(request(`/v1/sessions/${session}/append`,{}),env)).status,404);
  assert.equal((await sessionApi(request("/v1/sessions",{id:"../../other"}),env)).status,400);
  assert.equal((await sessionApi(request(`/v1/sessions/${session}`,undefined,AUTHORITY_TOKEN),env)).status,401);
  assert.equal(calls.length,0);
});
test("agent routes validate the authoritative agent binding before session access",async()=>{
  const {env,calls}=fixture();
  env.SESSIONS.get=name=>({fetch:async request=>{
    const call={name,...await request.json()};calls.push(call);
    if(call.operation==="scope" && call.body.agent_name!=="counter")return Response.json({error:"not found"},{status:404});
    return Response.json({name:"counter"});
  }});
  assert.equal((await sessionApi(request("/v1/channels/counter"),env)).status,200);
  assert.equal(calls[0].operation,"card");
  assert.equal((await sessionApi(request(`/v1/channels/other/sessions/${session}`),env)).status,404);
  assert.deepEqual(calls.slice(1).map(call=>call.operation),["scope"]);
  assert.equal((await sessionApi(request(`/v1/channels/counter/sessions/${session}`),env)).status,200);
  assert.deepEqual(calls.slice(2).map(call=>call.operation),["scope","status"]);
  assert.equal((await sessionApi(request("/v1/channels/counter/sessions",{id:session,agent_name:"other"}),env)).status,201);
  assert.equal(calls.at(-1).body.agent_name,"counter");
  assert.equal((await sessionApi(request("/v1/sessions"),env)).status,405);
});
test("cancellation acknowledges a targeted command and preserves retry identity",async()=>{
  const {env,calls}=fixture(); const command=crypto.randomUUID();
  const response=await sessionApi(request(`/v1/sessions/${session}/cancel`,{message_id:message,command_id:command}),env);
  assert.equal(response.status,202);
  assert.deepEqual(calls[0].body,{message_id:message,command_id:command});
});

test("Workers SSE replays exact cursors and closing the observer sends no cancel command",async()=>{
  const {env,calls}=fixture();
  env.SESSIONS.get=name=>({fetch:async request=>{
    if(new URL(request.url).pathname.endsWith("/watch"))return {status:101,webSocket:new WatchSocket()};
    const call={name,...await request.json()};calls.push(call);
    if(call.operation==="cursor") return Response.json({sequence:7});
    assert.equal(call.operation,"events");
    if(call.body.limit===1) return Response.json({events:[],next_cursor:null,high_watermark:9});
    assert.equal(call.body.after,7);
    return Response.json({events:[{id:"event_random",sequence:9,type:"turn.completed",data:{}}],next_cursor:null,high_watermark:9});
  }});
  const response=await sessionApi(request(`/v1/sessions/${session}/sse?since_id=event_opaque`),env);
  assert.equal(response.status,200);assert.equal(response.headers.get("content-type"),"text/event-stream");
  const reader=response.body.getReader();const decoder=new TextDecoder();
  assert.match(decoder.decode((await reader.read()).value),/event: connected/);
  const event=decoder.decode((await reader.read()).value);
  assert.match(event,/id: event_random/);assert.match(event,/event: turn.completed/);
  await reader.cancel();
  assert.ok(calls.every(call=>["cursor","events"].includes(call.operation)));
});

test("idle SSE waits for a notification instead of polling the session object",async()=>{
  const {env,calls}=fixture();const socket=new WatchSocket();let ready=false;
  env.SESSIONS.get=()=>({fetch:async request=>{
    if(new URL(request.url).pathname.endsWith("/watch")) {
      assert.equal(request.headers.get("authorization"),`Bearer ${AUTHORITY_TOKEN}`);
      assert.equal(request.headers.get("upgrade"),"websocket");
      assert.equal(request.headers.get("connection").toLowerCase(),"upgrade");
      assert.equal(request.headers.get("sec-websocket-version"),"13");
      assert.equal(Buffer.from(request.headers.get("sec-websocket-key"),"base64").length,16);
      return {status:101,webSocket:socket};
    }
    const call=await request.json();calls.push(call);
    return Response.json({events:ready?[{id:"event_wake",sequence:1,type:"turn.completed",data:{}}]:[],next_cursor:null,high_watermark:ready?1:0});
  }});
  const response=await sessionApi(request(`/v1/sessions/${session}/sse`),env);
  const reader=response.body.getReader();
  await reader.read();
  const next=reader.read();
  await new Promise(resolve=>setTimeout(resolve,650));
  assert.equal(calls.length,2,"one snapshot and one replay; no periodic object reads");
  ready=true;socket.change();
  assert.match(new TextDecoder().decode((await next).value),/id: event_wake/);
  await reader.cancel();
});

test("HTTP notification transport uses only the configured trusted origin",async context=>{
  const {env}=fixture();env.NOTIFICATION_ORIGIN="https://notifications.test";
  env.SESSIONS.get=()=>({fetch:async request=>{
    assert.ok(!new URL(request.url).pathname.endsWith("/watch"));
    return Response.json({events:[],next_cursor:null,high_watermark:0});
  }});
  let watched=false;
  context.mock.method(globalThis,"fetch",async request=>{
    watched=true;
    assert.equal(request.url,`https://notifications.test/tenants/test/sessions/${session}/watch`);
    assert.equal(request.redirect,"manual");
    assert.equal(request.headers.get("authorization"),`Bearer ${AUTHORITY_TOKEN}`);
    return {status:101,webSocket:new WatchSocket()};
  });
  const response=await sessionApi(new Request(`https://attacker.invalid/v1/sessions/${session}/sse`,{
    headers:{authorization:`Bearer ${API_TOKEN}`},
  }),env);
  assert.equal(response.status,200);assert.ok(watched);
  const reader=response.body.getReader();await reader.read();await reader.cancel();
});

test("HTTP notification redirects are rejected without forwarding authority credentials",async context=>{
  const {env}=fixture();env.NOTIFICATION_ORIGIN="https://notifications.test";
  env.SESSIONS.get=()=>({fetch:async()=>Response.json({events:[],next_cursor:null,high_watermark:0})});
  let calls=0;
  context.mock.method(globalThis,"fetch",async request=>{
    calls++;
    assert.equal(new URL(request.url).origin,"https://notifications.test");
    assert.equal(request.redirect,"manual");
    return new Response(null,{status:302,headers:{location:"https://attacker.invalid/steal"}});
  });
  const response=await sessionApi(request(`/v1/sessions/${session}/sse`),env);
  assert.equal(response.status,503);assert.equal(calls,1);
});
test("Workers event list preserves the native data envelope and total count",async()=>{
  const {env,calls}=fixture();
  env.SESSIONS.get=()=>({fetch:async request=>{calls.push(await request.json());return Response.json({data:[{id:"event_1"}],total:14});}});
  const response=await sessionApi(request(`/v1/sessions/${session}/events?limit=2&types=turn.completed`),env);
  assert.deepEqual(await response.json(),{data:[{id:"event_1"}]});
  assert.equal(response.headers.get("x-total-count"),"14");
  assert.equal(calls[0].operation,"list_events");assert.deepEqual(calls[0].body.types,["turn.completed"]);
});
