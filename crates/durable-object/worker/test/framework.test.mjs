import test from "node:test";
import assert from "node:assert/strict";
import { Authority } from "../src/authority.js";
import { Storage, sessionId, tenant, input, claim, event, start } from "./support.mjs";

async function fixture() {
  const storage = new Storage();
  let now = 1000000;
  const authority = new Authority(storage, () => now);
  const call = (operation, body = {}) => authority.handle(operation, body, sessionId, tenant);
  const binding = { agent: "counter", build_id: "framework-test", framework: 1 };
  await call("bind", { binding });
  return { storage, call, binding, advance: (ms) => { now += ms; } };
}

test("Framework binding and admission receipts reopen without frontend state", async () => {
  const f = await fixture();
  await f.call("bind", { binding: f.binding });
  await assert.rejects(f.call("bind", { binding: { ...f.binding, framework: 2 } }), /binding conflict/);
  const accepted = await f.call("input", { event: input() });
  assert.deepEqual(await f.call("receipt", { message_id: accepted.message_id }), accepted);
  const restarted = new Authority(f.storage, () => 1000001);
  assert.deepEqual(await restarted.handle("receipt", { message_id: accepted.message_id }, sessionId, tenant), accepted);
});

test("runtime termination retains a wakeup until Framework finalization commits", async () => {
  const f = await fixture();
  await f.call("input", { event: input() });
  const lease = await claim(f);
  const turn = `turn_${"2".repeat(32)}`;
  await start(f, lease, turn);
  const terminal = await f.call("append", { lease, event: event("turn.completed", turn, { turn_id: turn, iterations: 1 }) });
  await f.call("release", { ...lease, consumed_seq: terminal.event.sequence });
  assert.equal((await f.call("status")).runnable, true);
  assert.deepEqual((await f.call("status")).finalizing, [turn]);
  assert.notEqual(f.storage.alarm, null);
  // Recovery can finalize the closed turn without a new input or model call.
  const replacement = await claim(f, "replacement");
  assert.equal(await f.call("framework", { lease: replacement, operation: "set", key: `outcome/${turn}`, value: { success: true }, absent: true }), true);
  await f.call("release", { ...replacement, consumed_seq: terminal.event.sequence });
  assert.equal((await f.call("status")).runnable, false);
});

test("lifecycle invocation markers and outcomes reject expired epochs and unfenced frontends", async () => {
  const f = await fixture();
  await f.call("input", { event: input() });
  const lease = await claim(f);
  const write = { lease, operation: "set", key: "hook/agent_start/0", value: { state: "invoked" }, absent: true };
  assert.equal(await f.call("framework", write), true);
  assert.equal(await f.call("framework", write), false);
  await assert.rejects(f.call("framework", { ...write, lease: null }), /lease lost/);
  f.advance(10001);
  await assert.rejects(f.call("framework", { ...write, absent: false }), /lease lost/);
  assert.deepEqual(await f.call("framework", { operation: "get", key: write.key }), { state: "invoked" });
});

test("late targeted cancellation cannot cancel a newer turn", async () => {
  const f = await fixture();
  await f.call("input", { event: input() });
  const lease = await claim(f);
  const old = `turn_${"2".repeat(32)}`;
  const current = `turn_${"3".repeat(32)}`;
  await f.call("framework", { lease, operation: "set", key: `assignment/${input().data.message.id}`, value: { turn_id: old, steered: false } });
  await start(f, lease, current);
  await f.call("cancel", { command_id: "late-command", input_message_id: input().data.message.id });
  assert.equal((await f.call("status")).cancel_inbox_seq, null);
  await assert.rejects(f.call("cancel", { command_id: "late-command", input_message_id: input(2).data.message.id }), /different input/);
});

test("approval permission, correlated answer and wakeup commit atomically and retry after reply loss",async()=>{
  const f=await fixture(); await f.call("input",{event:input()});const lease=await claim(f);const turn=await start(f,lease);
  const request={id:"tool_approval_call_one",name:"approve_tool_call",arguments:{code:"tool_approval_required",error:"waiting",tool_call_id:"call_one",tool:"mutate",arguments:{n:1},fingerprint:"sha256:test",risk:"policy",mode:"normal",asked_at:new Date(1000000).toISOString(),expires_at:new Date(1100000).toISOString()}};
  const pause=await f.call("append",{lease,event:event("tool.call_requested",turn,{tool_calls:[request]})});
  await f.call("release",{...lease,consumed_seq:pause.event.sequence});
  assert.equal((await f.call("status")).runnable,false);
  const command={command_id:"approval-command",approvals:[{turn_id:turn,request_id:request.id,decision:"allow"}]};
  await f.call("approve",command);const history=await f.call("events");await f.call("approve",command);
  assert.deepEqual(await f.call("events"),history);
  const record=JSON.parse(await f.call("store",{kind:"value",operation:"get",key:"tool_approval/once/sha256_test"}));
  assert.equal(record.allow,true);assert.equal(record.fingerprint,request.arguments.fingerprint);
  assert.equal((await f.call("status")).pending.length,0);assert.equal((await f.call("status")).runnable,true);
  assert.equal(history.events.filter(event=>event.type==="tool.completed").length,1);
  await assert.rejects(f.call("approve",{...command,approvals:[{...command.approvals[0],decision:"reject"}]}),/different input/);
});

test("a resolution racing the parking owner's final cursor cannot erase its resume obligation",async()=>{
  const f=await fixture();await f.call("input",{event:input()});const lease=await claim(f);const turn=await start(f,lease);
  await f.call("append",{lease,event:event("tool.call_requested",turn,{tool_calls:[{id:"client_one",name:"client",arguments:{}}]})});
  await f.call("resolve",{turn_id:turn,resolution_id:"racing-resolution",results:[{tool_call_id:"client_one",tool_name:"client",success:true,status:"success",result:[{type:"text",text:"done"}]}]});
  const head=(await f.call("status")).head;
  const released=await f.call("release",{...lease,consumed_seq:head});
  assert.equal(released.Released.still_runnable,true,"the old owner never ran the continuation");
  assert.notEqual(await f.call("claim",{holder:"next-owner",ttl_ms:10000}),null);
});

test("a fenced parked deadline retains an independent alarm and becomes runnable without a new command",async()=>{
  const f=await fixture();await f.call("input",{event:input()});
  const lease=await claim(f,"timer-owner");const turn=await start(f,lease);f.storage.alarm=null;
  const pause=await f.call("append",{lease,event:event("tool.call_requested",turn,{tool_calls:[{id:"wait",name:"client",arguments:{}}]})});
  await f.call("schedule",{lease,wake_at:1000500});
  await f.call("release",{...lease,consumed_seq:pause.event.sequence});
  assert.equal((await f.call("status")).runnable,false);
  assert.equal(f.storage.alarm,1000500);
  await assert.rejects(f.call("schedule",{lease,wake_at:1000100}),/lease lost/);
  f.advance(501);
  const next=await claim(f,"timer-replacement");
  assert.ok(next);assert.ok(next.fence>lease.fence);
  assert.equal((await f.call("status")).wake_at,null);
});


test("authority derives approval writes in Rust and ignores caller-prepared permission records",async()=>{
  const f=await fixture();await f.call("input",{event:input()});const lease=await claim(f);const turn=await start(f,lease);
  const request={id:"tool_approval_call_one",name:"approve_tool_call",arguments:{code:"tool_approval_required",error:"waiting",tool_call_id:"call_one",tool:"mutate",arguments:{n:1},fingerprint:"sha256:test",risk:"policy",mode:"normal",asked_at:new Date(1000000).toISOString(),expires_at:new Date(1100000).toISOString()}};
  await f.call("append",{lease,event:event("tool.call_requested",turn,{tool_calls:[request]})});
  await f.call("approve",{command_id:"forged-preparation",approvals:[{turn_id:turn,request_id:request.id,decision:"reject"}],prepared:[{key:"tool_approval/always/mutate",record:JSON.stringify({tool:"mutate",allow:true})}]});
  assert.equal(await f.call("store",{kind:"value",operation:"get",key:"tool_approval/always/mutate"}),null);
  const record=JSON.parse(await f.call("store",{kind:"value",operation:"get",key:"tool_approval/once/sha256_test"}));
  assert.equal(record.allow,false);assert.equal(record.turn_id,turn);
});
