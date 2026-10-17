// Thin Workers HTTP mapping. Rust Engine/Session own all session semantics.
import { authenticate, failure, readJson } from "./http.js";
import { HttpError, requireValue } from "./storage.js";

const sessionPattern = /^session_[a-f0-9]{32}$/;
const messagePattern = /^message_[a-f0-9]{32}$/;
const identity = (prefix) => `${prefix}_${crypto.randomUUID().replaceAll("-", "")}`;

export async function sessionApi(request, env) {
  try {
    await authenticate(request, env.API_TOKEN);
    requireValue(env.API_TOKEN !== env.AUTHORITY_TOKEN, 503, "API and authority credentials must be distinct");
    const url = new URL(request.url);
    const route = url.pathname.match(/^\/v1\/sessions(?:\/(session_[a-f0-9]{32})(?:\/(messages|events|sse|cancel|tool-results|admissions|tool-approvals)(?:\/(message_[a-f0-9]{32}))?)?)?$/);
    requireValue(route, 404, "session route not found");
    let [,session,action,message] = route;
    const body = request.method === "POST" ? await readJson(request) : {};
    let operation, command, status = 200;
    if (!session && request.method === "POST") {
      session = body.id ?? body.session_id ?? identity("session");
      requireValue(sessionPattern.test(session),400,"invalid stable session identity");
      operation = "create"; command = {agent_name:body.agent_name,creation:{title:body.title??null,tags:body.tags??[],hints:body.hints??null,metadata:body.metadata??null}}; status = 201;
    } else if (!action && request.method === "GET") {
      operation = "status"; command = {};
    } else if (action === "messages" && request.method === "POST") {
      requireValue(body.message?.role == null || body.message.role === "user",400,"only user messages may be admitted");
      requireValue(!body.files?.length,400,"configure a durable workspace before attaching files");
      const input = body.message ?? body;
      message = body.message_id ?? input.id ?? identity("message");
      requireValue(messagePattern.test(message) && Array.isArray(input.content) && input.content.length>0,400,"invalid message");
      operation = "send"; command = {message_id:message,content:input.content}; status = 201;
    } else if (action === "events" && request.method === "GET") {
      const after=await replayPosition(request,env,session,url,0);
      operation = "list_events"; command = {after,limit:Number(url.searchParams.get("limit") ?? 1000),before:url.searchParams.has("before_sequence")?Number(url.searchParams.get("before_sequence")):null,types:url.searchParams.getAll("types"),exclude:url.searchParams.getAll("exclude")};
    } else if (action === "sse" && request.method === "GET") {
      return await eventStream(request,env,session,url);
    } else if (action === "admissions" && message && request.method === "GET") {
      operation = "admission"; command = {message_id:message};
    } else if (action === "cancel" && request.method === "POST") {
      operation = "cancel"; command = {message_id:body.message_id,command_id:body.command_id ?? crypto.randomUUID()}; status = 202;
    } else if (action === "tool-results" && request.method === "POST") {
      operation = "resolve"; command = {command_id:body.command_id ?? crypto.randomUUID(),turn_id:body.turn_id,results:body.results}; status = 202;
    } else {
      throw new HttpError(405,"method not supported");
    }
    const reply = await invoke(env,session,operation,command);
    if (!reply.ok) return reply;
    const data=await reply.json();
    if(operation==="list_events") return Response.json({data:data.data},{headers:{"x-total-count":String(data.total)}});
    return Response.json(data, {status,headers:operation==="create"?{location:`/v1/sessions/${session}`}:{}});
  } catch(error) { return failure(error); }
}

function invoke(env,session,operation,body) {
  const tenant = env.TENANT_ID;
  const stub = env.SESSIONS.get(env.SESSIONS.idFromName(`${tenant}:${session}`));
  return stub.fetch(new Request(`https://object.invalid/tenants/${tenant}/sessions/${session}/engine`, {
    method:"POST",headers:{authorization:`Bearer ${env.AUTHORITY_TOKEN}`,"content-type":"application/json"},body:JSON.stringify({operation,body}),
  }));
}

// Event ids are opaque. Exact lookup avoids assuming lexicographic UUID order
// and refuses another session's cursor. The poller never activates execution.
async function replayPosition(request,env,session,url,fallback) {
  const id=url.searchParams.get("since_id") ?? request.headers.get("last-event-id");
  if(id) return (await value(env,session,"cursor",{event_id:id})).sequence;
  const after=url.searchParams.get("after_sequence");
  if(after===null) return fallback;
  const sequence=Number(after);
  requireValue(Number.isSafeInteger(sequence) && sequence>=0,400,"invalid event sequence");
  return sequence;
}
async function value(env,session,operation,body) {
  const response=await invoke(env,session,operation,body);
  if(!response.ok) throw new HttpError(response.status,"session query failed");
  return response.json();
}
async function eventStream(request,env,session,url) {
  const snapshot=await value(env,session,"events",{after:0,limit:1});
  let after=await replayPosition(request,env,session,url,snapshot.high_watermark);
  let cursor=null, stopped=false, connected=false, heartbeat=Date.now();
  const types=url.searchParams.getAll("types"),exclude=url.searchParams.getAll("exclude");
  const encoder=new TextEncoder();
  const stop=()=>{stopped=true;};
  request.signal.addEventListener("abort",stop,{once:true});
  const stream=new ReadableStream({
    async pull(controller) {
      try {
        if(!connected) {connected=true;controller.enqueue(encoder.encode('event: connected\ndata: {"status":"connected"}\nretry: 1000\n\n'));return;}
        while(!stopped) {
          const page=await value(env,session,"events",cursor?{cursor,limit:64}:{after,limit:64});
          cursor=page.next_cursor;
          const output=[];
          for(const event of page.events) {
            after=Math.max(after,event.sequence);
            if((!types.length || types.includes(event.type)) && !exclude.includes(event.type)) output.push(`id: ${event.id}\nevent: ${event.type}\ndata: ${JSON.stringify(event)}\nretry: 1000\n\n`);
          }
          if(!cursor) after=Math.max(after,page.high_watermark);
          if(output.length) {controller.enqueue(encoder.encode(output.join("")));return;}
          if(Date.now()-heartbeat>=30000){heartbeat=Date.now();controller.enqueue(encoder.encode(": heartbeat\n\n"));return;}
          if(!cursor) await new Promise(resolve=>setTimeout(resolve,500));
        }
        controller.close();
      } catch(error) {controller.error(error);}
      finally {if(stopped)request.signal.removeEventListener("abort",stop);}
    },
    cancel(){stop();request.signal.removeEventListener("abort",stop);},
  },{highWaterMark:1});
  return new Response(stream,{headers:{"content-type":"text/event-stream","cache-control":"no-cache"}});
}
