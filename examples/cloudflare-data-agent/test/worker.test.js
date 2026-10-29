import { test } from "node:test";
import assert from "node:assert/strict";
import { authenticate, hmac, eventStream } from "../src/protocol.js";
import { execute, salesSummary } from "../src/index.js";
const secret = "test_endpoint_secret_01234567890123456789";
const id = "message_0123456789abcdef0123456789abcdef";
const body = () => ({ protocol: "everruns-worker/v1", audience:"https://agent.example/", type: "invoke", issued_at: Math.floor(Date.now()/1000), invocation_id: id, input_sequence: 1, input: { id, role: "user", content: [{type:"text",text:"North sales?"}] }, journal: [] });
async function signed(value, key = secret) {
  const bytes = new TextEncoder().encode(JSON.stringify(value));
  return new Request("https://agent.example/", { method:"POST", headers: {"X-Everruns-Signature": await hmac(key,bytes)}, body:bytes });
}

test("authenticity, expiry, protocol and missing secret fail closed", async () => {
  assert.equal((await authenticate(await signed(body()), secret)).invocation_id,id);
  await assert.rejects(authenticate(await signed(body(), "wrong"), secret));
  await assert.rejects(authenticate(await signed({...body(),issued_at:0}), secret));
  await assert.rejects(authenticate(await signed({...body(),protocol:"other"}), secret));
  await assert.rejects(authenticate(await signed({...body(),audience:"https://other.example/"}), secret));
  await assert.rejects(authenticate(await signed(body()), ""));
});

test("private query binds untrusted region and rejects malformed arguments", async () => {
  const database = {prepare(sql) { assert.match(sql,/WHERE region = \?/); return {bind(region) { assert.equal(region,"' OR 1=1 --"); return {first:async()=>({order_count:0,total_cents:0})};}};}};
  assert.deepEqual(await salesSummary(database,{region:"' OR 1=1 --"}),{order_count:0,total_cents:0});
  await assert.rejects(salesSummary(database,{}));
});

test("tool loop emits canonical events and usage; no CP credential reaches provider", async () => {
  const events = []; let calls = 0;
  const model = async (url,request) => {
    assert.equal(url,"https://api.openai.com/v1/responses");
    assert.equal(request.headers.Authorization,"Bearer app-owned-model-key");
    assert.equal(request.redirect,"manual");
    const sent=JSON.parse(request.body); assert.equal(sent.model,"gpt-6.1-sol");
    calls++;
    return Response.json({ model:sent.model, status:"completed", usage:{input_tokens:20,output_tokens:5}, output:calls===1?[{type:"function_call",call_id:"call_1",name:"sales_summary",arguments:'{"region":"north"}'}]:[{type:"message",role:"assistant",content:[{type:"output_text",text:"27278 cents, 2 orders"}]}] });
  };
  const database={prepare:()=>({bind:()=>({first:async()=>({order_count:2,total_cents:27278})})})};
  await execute(body(),{OPENAI_API_KEY:"app-owned-model-key",SALES:database},async(type,data)=>events.push({type,data}),id.replace("message_","turn_"),model);
  assert.deepEqual(events.map(e=>e.type),["llm.generation","tool.started","tool.completed","llm.generation","output.message.completed","turn.completed"]);
  assert.equal(events.at(-1).data.usage.input_tokens,40);
  assert.match(events.at(-2).data.message.content[0].text,/27278/);
});

test("stream errors become a failed turn and preserve input identity", async () => {
  const {response,task}=eventStream(body(),async()=>{throw new Error("secret diagnostic");});
  const lines=(await response.text()).trim().split("\n").map(JSON.parse); await task;
  assert.deepEqual(lines.map(e=>e.type),["turn.started","turn.failed"]);
  assert.equal(lines[1].context.input_message_id,id);
  assert.equal(lines[0].data.input_content,"North sales?");
  assert.doesNotMatch(JSON.stringify(lines),/secret diagnostic/);
});
