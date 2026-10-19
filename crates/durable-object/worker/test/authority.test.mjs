import test from "node:test";
import assert from "node:assert/strict";
import { Authority } from "../src/authority.js";
import {
  fixture,
  binding,
  sessionId,
  tenant,
  input,
  claim,
  event,
  start,
} from "./support.mjs";

test("acknowledged input and its alarm commit together, including after lost acknowledgement", async () => {
  const f = await fixture();
  f.storage.failAlarm = true;
  await assert.rejects(f.call("input", { event: input() }), /injected alarm/);
  assert.equal((await f.call("events")).events.length, 0);
  assert.equal((await f.call("status")).runnable, false);
  f.storage.failAlarm = false;
  const first = await f.call("input", { event: input() });
  assert.equal(f.storage.alarm, 1000010);
  const retry = input();
  retry.ts = "2027-01-01T00:00:00Z";
  assert.deepEqual(await f.call("input", { event: retry }), first);
  await assert.rejects(
    f.call("input", { event: input(1, "different") }),
    /different input/,
  );
  assert.equal((await f.call("events")).events.length, 1);
});

test("configuration patches and empty-history seeds retain retry results without activation",async()=>{
  const f=await fixture();const tools=[{type:"client_side",name:"render",description:"Show a result",parameters:{type:"object"}}];
  await f.call("configure",{command_id:"tools",patch:{client_tools:tools}});
  await f.call("configure",{command_id:"instructions",patch:{instructions:"Use the retained tools"}});
  assert.deepEqual((await f.call("snapshot")).configuration,{client_tools:tools,instructions:"Use the retained tools"});
  await f.call("configure",{command_id:"instructions",patch:{instructions:"Use the retained tools"}});
  await assert.rejects(f.call("configure",{command_id:"instructions",patch:{instructions:null}}),/different input/);
  await f.call("configure",{command_id:"clear",patch:{instructions:null}});
  assert.deepEqual((await f.call("snapshot")).configuration,{client_tools:tools,instructions:null});
  const events=[input()];
  assert.equal(await f.call("seed",{command_id:"import",events}),true);
  assert.equal(await f.call("seed",{command_id:"import",events}),true);
  assert.equal(await f.call("seed",{command_id:"another",events}),false);
  const snapshot=await f.call("snapshot");assert.equal(snapshot.high_watermark,1);
  assert.equal((await f.call("status")).runnable,false);assert.equal(f.storage.alarm,null);
});

test("already-cancelled admission commits cancellation with the input receipt",async()=>{
  const f=await fixture();const event=input();
  const receipt=await f.call("input",{event,cancelled:true});
  assert.equal((await f.call("status")).target_cancels[receipt.message_id],receipt.sequence);
  assert.deepEqual(await f.call("input",{event,cancelled:true}),receipt);
  await assert.rejects(f.call("input",{event,cancelled:false}),/different input/);
  assert.equal((await f.call("events")).events.length,1);
});

test("mixed approval and client results validate atomically and retain their receipt", async () => {
  const f = await fixture();
  await f.call("input", { event: input() });
  const owner = await claim(f);
  const turn = await start(f, owner);
  const approval = { id: "tool_approval_permission", name: "approve_tool_call", arguments: {code:"tool_approval_required",error:"waiting",tool_call_id:"permission",tool:"mutate",arguments:{n:1},fingerprint:"sha256:test",risk:"policy",mode:"normal",asked_at:new Date(1000000).toISOString(),expires_at:new Date(1010000).toISOString()} };
  const client = { id: "render", name: "render", arguments: {} };
  await f.call("append", { lease: owner, event: event("tool.call_requested", turn, { tool_calls: [approval, client] }) });
  await f.call("release", { ...owner, consumed_seq: 3 });
  const result = { tool_call_id: "render", tool_name: "render", success: true, status: "success", result: [] };
  const command = {
    command_id: "mixed", turn_id: turn,
    approvals: [{ turn_id: turn, request_id: approval.id, decision: "allow" }],
    results: [result],
  };
  const before = await f.call("events");
  await assert.rejects(f.call("resolve_waits", { ...command, results: [{ ...result, tool_call_id: "unknown" }] }), /pending call/);
  assert.deepEqual(await f.call("events"), before);
  assert.equal((await f.call("status")).pending.length, 2);
  await f.call("resolve_waits", command);
  const after = await f.call("events");
  assert.equal(after.events.length, before.events.length + 3);
  assert.equal((await f.call("status")).runnable, true);
  await f.call("resolve_waits", command);
  assert.deepEqual(await f.call("events"), after);
});

test("another authority instance reconstructs pending input from storage", async () => {
  const f = await fixture();
  await f.call("input", { event: input() });
  const restarted = new Authority(f.storage, () => 1001000);
  const claimed = await restarted.handle(
    "claim",
    { holder: "replacement", ttl_ms: 10000 },
    sessionId,
    tenant,
  );
  assert.equal(claimed.actor.inbox_seq, 1);
  assert.deepEqual(claimed.binding, binding);
});

test("ownership expiry fences starts, results, session state and late renewals", async () => {
  const f = await fixture();
  await f.call("input", { event: input() });
  const old = await claim(f);
  assert.equal(
    await f.call("claim", { holder: "duplicate", ttl_ms: 10000 }),
    null,
  );
  f.advance(10001);
  await assert.rejects(
    f.call("renew", { ...old, ttl_ms: 10000, consumed_seq: 0 }),
    /lease lost/,
  );
  const next = await claim(f, "worker-two");
  assert.ok(next.fence > old.fence);
  await assert.rejects(
    f.call("append", {
      lease: old,
      event: event("tool.started", "turn_a", { tool_call: { id: "call_1" } }),
    }),
    /lease lost/,
  );
  await assert.rejects(
    f.call("store", {
      lease: old,
      kind: "value",
      operation: "set",
      key: "x",
      value: "bad",
    }),
    /lease lost/,
  );
  await assert.rejects(
    f.call("release", { ...old, consumed_seq: 1 }),
    /lease lost/,
  );
});

test("input arriving at release remains runnable", async () => {
  const f = await fixture();
  await f.call("input", { event: input() });
  const owner = await claim(f);
  await f.call("input", { event: input(2) });
  assert.deepEqual(await f.call("release", { ...owner, consumed_seq: 1 }), {
    Released: { still_runnable: true },
  });
  const next = await f.call("claim", { holder: "worker-two", ttl_ms: 10000 });
  assert.equal(next.actor.consumed_seq, 1);
  assert.equal(next.actor.inbox_seq, 2);
});

test("cancellation retries cannot cancel the next generation", async () => {
  const f = await fixture({...binding,framework:1});
  await f.call("input", { event: input() });
  const owner = await claim(f);
  const turn=await start(f,owner);
  await f.call("framework",{lease:owner,operation:"set",key:`assignment/${input().data.message.id}`,value:{turn_id:turn},absent:true});
  await f.call("cancel", { command_id: "same-cancel", input_message_id: input().data.message.id });
  const renewal = await f.call("renew", {
    ...owner,
    ttl_ms: 10000,
    consumed_seq: 0,
  });
  assert.equal(renewal.cancel_inbox_seq, 1);
  const terminal=await f.call("append",{lease:owner,event:event("turn.cancelled",turn,{turn_id:turn})});
  await f.call("framework", {lease:owner,operation:"set",key:`outcome/${turn}`,value:{success:false}});
  await f.call("release", { ...owner, consumed_seq: terminal.event.sequence });
  await f.call("input", { event: input(2) });
  await f.call("cancel", { command_id: "same-cancel", input_message_id: input().data.message.id });
  const next = await f.call("claim", { holder: "worker-two", ttl_ms: 10000 });
  assert.equal(next.actor.cancel_inbox_seq, null);
});

test("client waits survive restart and partial resolution does not reactivate", async () => {
  const f = await fixture();
  await f.call("input", { event: input() });
  const owner = await claim(f);
  const turn = await start(f, owner);
  await f.call("append", {
    lease: owner,
    event: event("tool.call_requested", turn, {
      tool_calls: [
        { id: "a", name: "approve", arguments: {} },
        { id: "b", name: "approve", arguments: {} },
      ],
    }),
  });
  await f.call("release", { ...owner, consumed_seq: 3 });
  assert.equal((await f.call("status")).runnable, false);
  const result = (id) => ({
    tool_call_id: id,
    tool_name: "approve",
    success: true,
    status: "success",
    result: [],
  });
  const first = { resolution_id: "answer-a", results: [result("a")] };
  await f.call("resolve", first);
  assert.equal((await f.call("status")).runnable, false);
  const restarted = new Authority(f.storage, () => 1000010);
  await restarted.handle(
    "resolve",
    { resolution_id: "answer-b", results: [result("b")] },
    sessionId,
    tenant,
  );
  await f.call("resolve", first);
  assert.equal((await f.call("status")).runnable, true);
  assert.equal(
    (await f.call("events")).events.filter((e) => e.type === "tool.completed")
      .length,
    2,
  );
  await assert.rejects(
    f.call("resolve", { resolution_id: "unknown", results: [result("c")] }),
    /pending call/,
  );
});

test("operation results and terminal events cannot conflict", async () => {
  const f = await fixture();
  await f.call("input", { event: input() });
  const owner = await claim(f);
  const turn = await start(f, owner);
  const completed = event("tool.completed", turn, {
    tool_call_id: "a",
    tool_name: "write",
    success: true,
    result: [],
  });
  const first = await f.call("append", { lease: owner, event: completed });
  assert.deepEqual(
    await f.call("append", { lease: owner, event: completed }),
    first,
  );
  await assert.rejects(
    f.call("append", {
      lease: owner,
      event: { ...completed, data: { ...completed.data, success: false } },
    }),
    /conflicts/,
  );
  await f.call("append", {
    lease: owner,
    event: event("turn.completed", turn, { turn_id: turn }),
  });
  await assert.rejects(
    f.call("append", {
      lease: owner,
      event: event("turn.cancelled", turn, { turn_id: turn }),
    }),
    /conflicts/,
  );
});

test("conditional appends and bounded snapshot cursors retain canonical order", async () => {
  const f = await fixture();
  await f.call("input", { event: input() });
  const owner = await claim(f);
  const conflict = await f.call("append", {
    lease: owner,
    conditional: true,
    expected: null,
    event: event("session.started", null, {}),
  });
  assert.equal(conflict.conflict, true);
  assert.equal((await f.call("status")).head, 1);
  const page = await f.call("events", { limit: 1 });
  await f.call("input", { event: input(2) });
  assert.equal(
    (await f.call("events", { after: 1, through: page.high_watermark })).events
      .length,
    0,
  );
  assert.equal((await f.call("events", { after: 1 })).events.length, 1);
  await assert.rejects(
    f.call("events", { after: 0, limit: 2000 }),
    /cursor or limit/,
  );
  await assert.rejects(
    f.call("events", { after: 3, through: 2 }),
    /cursor or limit/,
  );
});

test("agent bindings cannot change and tenant/session identities are isolated", async () => {
  const f = await fixture();
  await assert.rejects(
    f.call("bind", { binding: { ...binding, build_id: "another-build" } }),
    /binding conflict/,
  );
  await assert.rejects(
    f.authority.handle("status", {}, sessionId, "other-tenant"),
    /not found/,
  );
  const bad = input();
  bad.session_id = `session_${"3".repeat(32)}`;
  await assert.rejects(
    f.call("input", { event: bad }),
    /invalid canonical event/,
  );
  assert.equal((await f.call("status")).head, 0);
});

test("large event pages stream within their byte budget and preserve a snapshot", async () => {
  const f = await fixture();
  const text = "x".repeat(800000);
  for (let i = 1; i <= 4; i++) await f.call("input", { event: input(i, text) });
  const first = await f.call("events", { limit: 256 });
  assert.equal(first.events.length, 2);
  assert.equal(first.has_more, true);
  const second = await f.call("events", {
    after: first.events.at(-1).sequence,
    through: first.high_watermark,
    limit: 256,
  });
  assert.equal(second.events.length, 2);
  assert.equal(second.has_more, false);
});

test("checkpoint lookup keeps formats separate and chooses the newest matching source", async () => {
  const f = await fixture();
  await f.call("input", { event: input() });
  const lease = await claim(f);
  for (const [format, source] of [
    [1, 10],
    [2, 20],
  ]) {
    const value = {
      session_id: sessionId,
      provider_type: "openai",
      model: "m",
      format_version: format,
      source_sequence: source,
    };
    assert.equal(
      await f.call("store", {
        lease,
        kind: "checkpoint",
        operation: "install",
        key: JSON.stringify(["openai", "m", format]),
        value,
      }),
      true,
    );
  }
  const read = (value) =>
    f.call("store", {
      kind: "checkpoint",
      operation: "latest",
      key: JSON.stringify(["openai", "m"]),
      value,
    });
  assert.equal((await read(null)).source_sequence, 20);
  assert.equal((await read(1)).source_sequence, 10);
  assert.equal(await read(3), null);
});
