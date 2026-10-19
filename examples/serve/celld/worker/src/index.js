// One authority DO per tenant/session. Containers are replaceable execution
// hosts only: they do not restore canonical snapshots or replay HTTP requests.
import { Container } from "@cloudflare/containers";
import authorityWorker, { SessionAuthority as BaseAuthority, authenticate, failure } from "../../../../../crates/durable-object/worker/src/index.js";
import { runAlarm } from "../../../../../crates/durable-object/worker/src/activation.js";

function executor(env) {
  return env.EXECUTORS.get(env.EXECUTORS.idFromName(`${env.TENANT_ID}:app`));
}
export class SessionAuthority extends BaseAuthority {
  async alarm() {
    if (this.env.SERVE_URL) return runAlarm(this.authority,{...this.env,EXECUTOR_URL:new URL("/execute",this.env.SERVE_URL).href});
    return runAlarm(this.authority,{...this.env,EXECUTOR_URL:"https://executor.internal/execute"},(url,init)=>executor(this.env).fetch(new Request(url,init)));
  }
}

export class ServeExecutor extends Container {
  defaultPort=8080;
  sleepAfter="10m";
  constructor(ctx,env) {
    super(ctx,env);
    const names=new Set(["AUTHORITY_URL","AUTHORITY_TOKEN","TENANT_ID","EXECUTOR_TOKEN","SERVE_WORKSPACE_DIR",...(env.CONTAINER_ENV??"").split(",").map(name=>name.trim())]);
    this.envVars=Object.fromEntries(Object.entries(env).filter(([name,value])=>typeof value==="string" && (name.startsWith("SERVE_") || names.has(name))));
    this.envVars.SERVE_API_KEYS=env.API_TOKEN;
  }
  fetch(request) {return this.containerFetch(request);}
}
export default {
  async fetch(request,env) {
    const url=new URL(request.url);
    if(url.pathname.startsWith("/tenants/")) return authorityWorker.fetch(request,env);
    try {
      await authenticate(request,env.API_TOKEN);
      if(!url.pathname.startsWith("/v1/") && url.pathname!=="/health") return new Response("not found",{status:404});
      if(env.API_TOKEN===env.AUTHORITY_TOKEN || env.API_TOKEN===env.EXECUTOR_TOKEN) return new Response("hosting credentials must be distinct",{status:503});
      if(env.SERVE_URL) return fetch(new Request(new URL(url.pathname+url.search,env.SERVE_URL),request),{redirect:"manual"});
      return executor(env).fetch(request);
    } catch(error) {return failure(error);}
  },
};
