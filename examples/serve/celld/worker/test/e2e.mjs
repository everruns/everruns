// Test-only model + real celld authority + replaceable native serve process.
// Canonical sessions are observed through the ordinary authenticated /v1 API.
import assert from "node:assert/strict";
import {spawn,execFileSync} from "node:child_process";
import {createServer} from "node:http";
import {mkdtempSync,writeFileSync,openSync,closeSync,existsSync,unlinkSync} from "node:fs";
import {tmpdir} from "node:os";
import {join,resolve,dirname} from "node:path";
import {fileURLToPath} from "node:url";
const here=dirname(fileURLToPath(import.meta.url)),worker=resolve(here,".."),repo=resolve(worker,"../../../..");
const run=mkdtempSync(join(tmpdir(),"everruns-serve-celld-engine."));
const binary=process.env.SERVE_BIN??join(repo,"target/debug/celld-agent");
const containerImage=process.env.SERVE_TEST_CONTAINER_IMAGE;
const externalAuthority=process.env.SERVE_TEST_AUTHORITY_URL;
assert.ok(existsSync(binary),"build first: cargo build -p serve-example-celld");
const token="synthetic-authority-credential-at-least-32-bytes",apiToken="synthetic-api-credential-at-least-32-bytes",executorToken="synthetic-executor-credential-at-least-32-bytes";
const children=[];const servers=[];
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
// Ports handed out by freePort are released before their owner binds them (the
// native executor binds SERVE_TEST_NATIVE_PORT long after a qualification parent
// picked it), so a later port-0 listener can be given the same port and receive
// the owner's traffic. Every listener here skips ports already reserved.
const reserved=new Set([Number(process.env.SERVE_TEST_NATIVE_PORT)].filter(Boolean));
async function listen(server){servers.push(server);for(;;){await new Promise((resolve,reject)=>server.once("error",reject).listen(0,"127.0.0.1",resolve));const port=server.address().port;if(!reserved.has(port)){reserved.add(port);return port;}await new Promise(resolve=>server.close(resolve));}}
async function freePort(){const server=createServer();const port=await listen(server);await new Promise(resolve=>server.close(resolve));return port;}
function start(command,args,name,extra={}) {
 const fd=openSync(join(run,`${name}.log`),"w");const child=spawn(command,args,{cwd:repo,env:{...process.env,...extra},detached:true,stdio:["ignore",fd,fd]});closeSync(fd);
 child.failure=null;child.once("error",error=>child.failure=error);children.push(child);return child;
}
async function kill(child){if(child.containerName){try{execFileSync("docker",["rm","--force",child.containerName],{stdio:"ignore"});}catch{}child.containerName=null;}const exited=child.exitCode!==null||child.signalCode!==null;const exit=exited?Promise.resolve():new Promise(resolve=>child.once("exit",resolve));try{process.kill(-child.pid,"SIGKILL");}catch{}await exit;}
async function until(label,fn,timeout=30000){const deadline=Date.now()+timeout;while(Date.now()<deadline){if(providerFailure)throw providerFailure;for(const child of children)if(child.failure)throw child.failure;try{const value=await fn();if(value)return value;}catch(error){if(error.name!=="TypeError")throw error;}await sleep(100);}throw new Error(`${label}: ${run}`);}
const nativePort=Number(process.env.SERVE_TEST_NATIVE_PORT??await freePort()),authorityPort=await freePort();
const authority=externalAuthority??`http://127.0.0.1:${authorityPort}`,native=`http://127.0.0.1:${nativePort}`;
let modelCalls=0,providerFailure=null;
const gatewayPort=await listen(createServer(async(request,response)=>{
 try {
  let raw="";for await(const chunk of request)raw+=chunk;
  const input=JSON.parse(raw);modelCalls++;
  assert.ok(Array.isArray(input.input),"native OpenAI Responses request expected");
  const done=input.input.some(item=>item.type==="function_call_output");
  response.writeHead(200,{"content-type":"text/event-stream"});
  const emit=event=>response.write(`data: ${JSON.stringify(event)}\n\n`);
  const item=done?{type:"message",id:"msg_recovered",status:"completed",role:"assistant",content:[{type:"output_text",text:"Recovered through Framework Engine.",annotations:[]}]}:{type:"function_call",id:"fc_research",call_id:"call_research",name:"look_up",arguments:'{"topic":"celld"}',status:"completed"};
  emit({type:"response.output_item.added",sequence_number:0,output_index:0,item});
  if(done) emit({type:"response.output_text.delta",sequence_number:1,item_id:item.id,output_index:0,content_index:0,delta:item.content[0].text});
  emit({type:"response.output_item.done",sequence_number:2,output_index:0,item});
  emit({type:"response.completed",sequence_number:3,response:{id:`response_${modelCalls}`,object:"response",created_at:0,model:input.model,status:"completed",output:[item],usage:{input_tokens:10,output_tokens:2,total_tokens:12}}});
  response.end();
 } catch(error) {providerFailure=error;if(!response.headersSent)response.writeHead(500);response.end();}

}));
const config=join(run,"wrangler.json");
writeFileSync(join(run,"worker.js"),`export {default,SessionAuthority,ServeExecutor} from ${JSON.stringify(join(worker,"src/index.js"))};\n`);
writeFileSync(config,JSON.stringify({name:"serve-celld-engine-test",main:"worker.js",compatibility_date:"2026-09-01",compatibility_flags:["nodejs_compat"],vars:{TENANT_ID:"test",SERVE_URL:native,AUTHORITY_TOKEN:token,EXECUTOR_TOKEN:executorToken,API_TOKEN:apiToken},durable_objects:{bindings:[{name:"SESSIONS",class_name:"SessionAuthority"}]},migrations:[{tag:"v1",new_sqlite_classes:["SessionAuthority"]}]}));
const nativeEnv={AUTHORITY_URL:authority,TENANT_ID:"test",AUTHORITY_TOKEN:token,EXECUTOR_TOKEN:executorToken,SERVE_API_KEYS:apiToken,SERVE_GATEWAY_URL:`http://127.0.0.1:${gatewayPort}/v1`,SERVE_GATEWAY_KEY:"synthetic",RESEARCH_DELAY_MS:process.env.SERVE_TEST_TOOL_DELAY_MS??"120000",PORT:String(nativePort)};
let generation=0;
function startNative(){
 const name=`native-${++generation}`,env={...nativeEnv,SERVE_DATA_DIR:join(run,name)};
 if(!containerImage)return start(binary,["celld"],name,env);
 const containerName=`er-engine-qual-${process.pid}-${generation}`;
 const args=["run","--rm","--name",containerName,"--network","host","--mount",`type=bind,source=${binary},target=/usr/local/bin/celld-agent,readonly`,"--mount",`type=bind,source=${run},target=${run}`];
 // The minimal qualification image uses the host's public trust roots.
 if(existsSync("/etc/ssl/certs"))args.push("--mount","type=bind,source=/etc/ssl/certs,target=/etc/ssl/certs,readonly");
 for(const key of Object.keys(env))args.push("--env",key);
 args.push("--entrypoint","/usr/local/bin/celld-agent",containerImage,"celld");
 const child=start("docker",args,name,env);child.containerName=containerName;return child;
}
async function api(path,method="GET",body) {
 const response=await fetch(`${authority}/v1${path}`,{method,headers:{authorization:`Bearer ${apiToken}`,"content-type":"application/json"},...(body===undefined?{}:{body:JSON.stringify(body)}),signal:AbortSignal.timeout(15000)});
 const value=await response.json();assert.ok(response.ok,`${method} ${path}: ${response.status} ${JSON.stringify(value)}`);return {status:response.status,value};
}
let cleanupPromise;
function cleanup(){return cleanupPromise??=(async()=>{
 for(const child of [...children].reverse())await kill(child);
 for(const server of servers){server.closeAllConnections?.();await new Promise(resolve=>server.close(resolve));}
 if(existsSync(config))unlinkSync(config);
})();}
// A fleet harness timeout terminates this driver. Clean its detached executor
// and Docker container as well as the driver's own process.
for(const [signal,code] of [["SIGTERM",143],["SIGINT",130]])process.once(signal,()=>{void cleanup().finally(()=>process.exit(code));});
try {
 if(!externalAuthority)start("celld",["dev",config,"--port",String(authorityPort),"--no-watch","--logs"],"celld",{CELLD_ESBUILD:join(repo,"crates/durable-object/worker/node_modules/esbuild/bin/esbuild")});
 let executor=startNative();
 await until("native ready",async()=>{const response=await fetch(`${native}/health`,{headers:{authorization:`Bearer ${apiToken}`}});return response.ok;});
 await until("authority ready",async()=>{const response=await fetch(authority);return response.status===401;});
 const session=`session_${crypto.randomUUID().replaceAll("-","")}`,message=`message_${crypto.randomUUID().replaceAll("-","")}`;
 const create={id:session};const created=await api("/sessions","POST",create);
 assert.deepEqual((await api("/sessions","POST",create)).value,created.value);
 const send={message_id:message,message:{role:"user",content:[{type:"text",text:"Research celld."}]}};
 const accepted=await api(`/sessions/${session}/messages`,"POST",send);
 assert.deepEqual((await api(`/sessions/${session}/messages`,"POST",send)).value,accepted.value);
 await until("tool started",async()=>(await api(`/sessions/${session}/events`)).value.data.some(event=>event.type==="tool.started"));
 if(process.env.SERVE_TEST_OWNER_LOSS_SIGNAL){
   const signal=process.env.SERVE_TEST_OWNER_LOSS_SIGNAL;writeFileSync(signal,JSON.stringify({session,tenant:"test"}));
   await until("fleet owner killed",async()=>existsSync(`${signal}.continue`),90000);
 }
 const second=`session_${crypto.randomUUID().replaceAll("-","")}`;
 assert.equal((await api("/sessions","POST",{id:second})).value.id,second,"creation commits while another session executes");
 if(process.env.SERVE_TEST_KILL_EXECUTOR!=="false"){await kill(executor);executor=startNative();}
 // No API requests during recovery. Startup is immediate; DO alarms own retry.
 await sleep(Number(process.env.SERVE_TEST_NO_CLIENT_MS??20000));
 const outcome=(await api(`/sessions/${session}/admissions/${message}`)).value;
 assert.equal(outcome.status,"complete");assert.equal(outcome.turn.success,true);
 const events=(await api(`/sessions/${session}/events`)).value.data;
 for(const kind of ["input.message","turn.started","tool.started","tool.completed","turn.completed"])assert.equal(events.filter(event=>event.type===kind).length,1,kind);
 const toolStatus=events.find(event=>event.type==="tool.completed").data.status;
 assert.ok((process.env.SERVE_TEST_KILL_EXECUTOR==="false"?["success","interrupted"]:["interrupted"]).includes(toolStatus),toolStatus);
 assert.equal(modelCalls,2,"committed reason and tool dispatch must not repeat");
 assert.equal((await api(`/sessions/${second}`)).value.id,second,"empty session survives catalog loss too");
 console.log(JSON.stringify({backend:"celld",placement:containerImage?"native serve Engine in Docker":"native serve Engine process",creation_while_busy:"committed",recovery:"independent alarm",canonical_replay:false,model_calls:modelCalls,admissions:1,turns:1,tool_status:toolStatus,authority_owner_loss:!!process.env.SERVE_TEST_OWNER_LOSS_SIGNAL}));
 console.log(`Native serve qualification evidence: ${run}`);
} finally {
 await cleanup();
}
