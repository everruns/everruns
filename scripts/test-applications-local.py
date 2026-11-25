#!/usr/bin/env python3
"""Real API + PostgreSQL + HTTPS publication host acceptance test.

Build chat-ui and the chat-capacity-fixture binary first.
The shared isolated fixture primes only local IdP discovery/JWKS; production
signature, audience, binding and resource authorization remain enabled. The
interactive OIDC handoff is covered separately by chat-ui's test:broker.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

import httpx

sys.path.insert(0, str(Path(__file__).resolve().parent / "capacity"))
from local_setup import Lab, ROOT, private_json

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--run-dir", required=True)
parser.add_argument("--port-prefix", default="272")
parser.add_argument("--fixture-bin", type=Path, default=ROOT / "target/debug/chat-capacity-fixture")
parser.add_argument("--attach-chats", action="store_true", help="Exercise attaching existing Work Chats instead of atomic Chat creation")
args = parser.parse_args()
args.users = 1
args.worker_slots = 2
args.max_running_turns = 2
args.worker_bin = ROOT / "target/debug/everruns-worker"
lab = Lab(args)
try:
    lab.start_infra()
    lab.start_identity()
    lab.env.update(CHAT_DOMAIN="capacity.localhost", CHAT_OIDC_ISSUER=lab.issuer,
                   CHAT_OIDC_CLIENT_ID="capacity-bench")
    lab.start_platform(start_worker=False)
    # Chromium uses the production origin, including standard HTTPS port.
    lab.proxy = 443
    lab.edge = "https://127.0.0.1"
    lab.start_host_and_proxy()
    credentials = json.loads(lab.users_file.read_text())
    control = httpx.Client(base_url=lab.api, trust_env=False, timeout=60,
        headers={"Authorization": "Bearer " + credentials["admin"]["access_token"]})
    def command(method, path, body=None):
        response = control.request(method, "/v1/platform/" + path, json=body)
        assert response.is_success, (method, path, response.status_code, response.text[:1000])
        return response.json()
    org = command("POST", "orgs", {"name":"Application acceptance"})
    control.headers["X-Org-Id"] = org["id"]
    agents = {
        "writer": command("POST", "agents", {"name":"writing-assistant", "system_prompt":"Help the editorial team write."}),
        "reviewer": command("POST", "agents", {"name":"review-assistant", "system_prompt":"Review the editorial team’s work."}),
    }
    app = command("POST", "applications", {"name":"Content Studio", "slug":"content-studio",
        "description":"Plan, write and review together. Shared knowledge keeps every conversation in context.",
        "config":{"membership":"open", "show_in_sidebar":True},"resources":[]})
    path = "applications/" + app["id"]
    for kind, name, slug in [("chat","Writing assistant","writer"),("chat","Review assistant","reviewer"),
                            ("room","Editorial room","editorial"),("memory","Brand handbook","handbook")]:
        agent = agents.get(slug)
        if kind == "chat" and args.attach_chats:
            channel_path = "agents/" + agent["id"] + "/channels"
            channel = command("POST", channel_path, {"channel_type":"work_chat",
                "channel_config":{"execution":{"session_binding":"per_user","visibility":"full"}}})
            command("POST", channel_path+"/"+channel["id"]+"/publish", {})
            app["resources"].append({"kind":kind,"name":name,"slug":slug,"resource_id":channel["id"],"writable":False})
            app = command("PATCH",path,{key:app[key] for key in ["name","slug","description","config","resources","version"]})
            continue
        app = command("POST", path+"/resources", {"version":app["version"],"kind":kind,"name":name,
            "slug":slug, **({"agent_id":agent["id"]} if kind == "chat" else {})})
    app["resources"][-1]["writable"] = True
    fields = ["name", "slug", "description", "config", "resources", "version"]
    app = command("PATCH",path,{key:app[key] for key in fields})
    app = command("POST",path+"/status",{"version":app["version"],"status":"published"})
    with httpx.Client(verify=str(lab.cert_path), trust_env=False) as identity:
        session = identity.post(lab.idp_url+"/provision", headers={"X-Capacity-Secret":lab.fixture_secret},
            json={"subject":"editorial-alice","channel":app["id"]}).raise_for_status().json()
    manifest = lab.private_dir / "browser.json"
    private_json(manifest, {"origin":"https://content-studio.capacity.localhost", "cookie":session["cookie"],
                           "results":str(lab.results)})
    subprocess.run(["node", "e2e/applications.local.mjs", str(manifest)], cwd=ROOT/"apps/chat-ui", check=True)
    # Publication revocation applies through the same encrypted browser session.
    command("POST",path+"/status",{"version":app["version"],"status":"draft"})
    with httpx.Client(verify=str(lab.cert_path), trust_env=False) as consumer:
        denied=consumer.get(lab.edge+"/api/application/manifest",
            headers={"Host":"content-studio.capacity.localhost","Cookie":session["cookie"]})
        assert denied.status_code in (401,403,404), denied.status_code
    print("PASS: real Application publication, multi-chat sessions, Room post/reply, memory persistence, CSRF, unpublish revocation")
finally:
    lab.stop()
