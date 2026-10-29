"""First-class local chat capacity bench: synthetic users, llmsim, real platform.

Smoke is the default. --qualification runs stepped steady load, a 30-minute
1,000-user soak, a simultaneous-turn burst and replay/reconnect qualification.
All output is local; private/ contains credentials and must never be published.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import resource
import re
from pathlib import Path
import subprocess
import sys
import time
import uuid

import httpx

from .local_setup import Lab, ROOT, hardware, private_json
from .merge import merge
from .run import verdict
from .resources import summarize


def build(args):
    env = {
        **os.environ,
        "CARGO_BUILD_JOBS": str(args.build_jobs),
        "CARGO_INCREMENTAL": "0",
        "CARGO_PROFILE_RELEASE_LTO": "false",
        "CARGO_PROFILE_RELEASE_CODEGEN_UNITS": "16",
        "CARGO_PROFILE_RELEASE_DEBUG": "0",
        "CARGO_PROFILE_RELEASE_OPT_LEVEL": "3",
    }
    Path(args.build_log).parent.mkdir(parents=True, exist_ok=True)
    with open(args.build_log, "w") as log:

        def artifact(package, name, extra):
            process = subprocess.Popen(
                [
                    "cargo",
                    "build",
                    "--release",
                    "-p",
                    package,
                    *extra,
                    "--message-format=json",
                ],
                cwd=ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stderr=log,
                text=True,
            )
            executable = None
            for line in process.stdout:
                try:
                    row = json.loads(line)
                    if row.get("reason") == "compiler-message":
                        log.write(row["message"].get("rendered", ""))
                    if (
                        row.get("reason") == "compiler-artifact"
                        and row.get("target", {}).get("name") == name
                    ):
                        executable = row.get("executable")
                except json.JSONDecodeError:
                    log.write(line)
            if process.wait() != 0 or not executable:
                raise RuntimeError("capacity " + name + " build failed; see build log")
            return Path(executable)

        args.fixture_bin = artifact(
            "everruns-server",
            "chat_capacity_fixture",
            ["--bench", "chat_capacity_fixture"],
        )
        args.worker_bin = artifact(
            "everruns-worker", "everruns-worker", ["--bin", "everruns-worker"]
        )
    subprocess.run(
        ["pnpm", "install", "--frozen-lockfile"],
        cwd=ROOT / "apps/chat-ui",
        check=True,
        stdout=subprocess.DEVNULL,
    )
    subprocess.run(
        ["pnpm", "build"],
        cwd=ROOT / "apps/chat-ui",
        check=True,
        stdout=subprocess.DEVNULL,
    )


def bootstrap(lab):
    actors = []
    with httpx.Client(timeout=60, trust_env=False) as client:
        identity_fixture = json.loads(lab.users_file.read_text())
        headers = {
            "Authorization": "Bearer " + identity_fixture["admin"]["access_token"]
        }

        def api(path, method="GET", data=None, org=None):
            result = client.request(
                method,
                lab.api + path,
                headers={**headers, **({"X-Org-Id": org} if org else {})},
                json=data,
            )
            if not result.is_success:
                raise RuntimeError(
                    f"fixture setup {method} {path}: HTTP {result.status_code}"
                )
            return result.json()

        users = identity_fixture["users"]
        fixtures = []
        for index in range(lab.args.tenants):
            org = api("/v1/orgs", "POST", {"name": f"Capacity tenant {index}"})
            org_id = org.get("public_id", org.get("id"))
            if not isinstance(org_id, str) or not re.fullmatch(
                r"org_[0-9a-f]{32}", org_id
            ):
                raise RuntimeError("invalid synthetic organization identifier")
            internal = int(
                lab.psql(
                    f"SELECT org_id FROM organizations WHERE public_id='{org_id}';"
                ).strip()
            )
            sql = []
            for i, user in enumerate(users):
                if i % lab.args.tenants == index:
                    # Identifiers are UUIDs from our own production storage fixture.
                    uid = str(uuid.UUID(user["id"]))
                    sql.append(
                        f"INSERT INTO organization_members(org_id,user_id,role) VALUES({internal},'{uid}','member');"
                    )
            lab.psql("\n".join(sql))
            provider = api(
                "/v1/providers",
                "POST",
                {"name": "Capacity simulator", "provider_type": "llmsim"},
                org_id,
            )
            model = api(
                f"/v1/providers/{provider['id']}/models",
                "POST",
                {
                    "model_id": f"llmsim-capacity-{index}",
                    "display_name": "Capacity simulator",
                    "capabilities": ["chat", "tools"],
                    "enabled": True,
                },
                org_id,
            )
            harnesses = api("/v1/harnesses", org=org_id)["data"]
            harness = next(
                (h for h in harnesses if h.get("name") == "base"), harnesses[0]
            )
            agent = api(
                "/v1/agents",
                "POST",
                {
                    "name": "capacity-agent",
                    "system_prompt": "Use only local test files.",
                    "harness_id": harness["id"],
                    "default_model_id": model["id"],
                    "communication": "direct",
                    "capabilities": [{"ref": "session_file_system"}],
                },
                org_id,
            )
            api_channel = api(
                f"/v1/agents/{agent['id']}/channels",
                "POST",
                {
                    "channel_type": "api",
                    "enabled": True,
                    "channel_config": {"visibility": "full"},
                },
                org_id,
            )
            api(
                f"/v1/agents/{agent['id']}/channels/{api_channel['id']}/publish",
                "POST",
                {},
                org_id,
            )
            key = api(
                f"/v1/agents/{agent['id']}/channels/{api_channel['id']}/keys",
                "POST",
                {"name": "capacity fixture", "permissions": ["sessions", "end_user"]},
                org_id,
            )["secret"]
            execution = {
                "visibility": "full",
                "auth_methods": [
                    {
                        "mode": "oidc",
                        "provider": {"type": "oidc", "issuer": lab.issuer},
                        "requirements": {"audiences": ["capacity-bench"]},
                    }
                ],
            }
            chat = api(
                f"/v1/agents/{agent['id']}/channels",
                "POST",
                {
                    "channel_type": "work_chat",
                    "enabled": True,
                    "channel_config": {
                        "interface_mode": "multi_chat",
                        "execution": execution,
                    },
                },
                org_id,
            )
            api(
                f"/v1/agents/{agent['id']}/channels/{chat['id']}/publish",
                "POST",
                {},
                org_id,
            )
            # Prove that priming local public keys preserves cryptographic checks.
            with httpx.Client(verify=str(lab.cert), trust_env=False) as idp:
                for variant in ["valid", "audience", "expired", "signature"]:
                    token = (
                        idp.post(
                            lab.issuer + "/verification-fixture",
                            headers={"X-Capacity-Secret": lab.fixture_secret},
                            json={"variant": variant},
                        )
                        .raise_for_status()
                        .json()["token"]
                    )
                    admitted = client.post(
                        lab.api + f"/v1/channels/{chat['id']}/runtime-auth",
                        headers={"Authorization": "Bearer " + token},
                    )
                    if (variant == "valid" and not admitted.is_success) or (
                        variant != "valid" and admitted.status_code not in {401, 403}
                    ):
                        raise RuntimeError(
                            "local issuer verification invariant failed: " + variant
                        )
            fixtures.append(
                {
                    "org": org_id,
                    "model_id": model["id"],
                    "agent_id": agent["id"],
                    "harness_id": harness["id"],
                    "api_channel": api_channel["id"],
                    "chat_channel": chat["id"],
                    "agent_key": key,
                }
            )
        for i, user in enumerate(users):
            group = fixtures[i % len(fixtures)]
            common = {
                "org": group["org"],
                "model_id": group["model_id"],
                "agent_id": group["agent_id"],
            }
            actors.append(
                {
                    **common,
                    "name": f"console-{i}",
                    "path": "console",
                    "base": lab.edge + "/api",
                    "headers": {
                        "Host": "console.capacity.localhost",
                        "Origin": "https://console.capacity.localhost",
                        "Cookie": user["cookie"],
                        "X-Org-Id": group["org"],
                    },
                    "create": {
                        "harness_id": group["harness_id"],
                        "agent_id": group["agent_id"],
                        "model_id": group["model_id"],
                        "source": "chat",
                    },
                }
            )
            actors.append(
                {
                    **common,
                    "name": f"agent-api-{i}",
                    "path": "agent-api",
                    "base": lab.edge + f"/api/v1/channels/{group['api_channel']}",
                    "channel_id": group["api_channel"],
                    "headers": {
                        "Authorization": "Bearer " + group["agent_key"],
                        "End-User": f"capacity-{i}",
                    },
                }
            )
            chat_host = (
                group["chat_channel"].removeprefix("appchan_") + ".capacity.localhost"
            )
            with httpx.Client(
                verify=str(lab.cert), trust_env=False, timeout=30
            ) as identity_client:
                opaque = identity_client.post(
                    lab.issuer + "/provision",
                    headers={"X-Capacity-Secret": lab.fixture_secret},
                    json={"subject": f"capacity-{i}", "channel": group["chat_channel"]},
                )
                opaque.raise_for_status()
            actors.append(
                {
                    **common,
                    "name": f"work-chat-{i}",
                    "path": "work-chat",
                    "base": lab.edge + "/api",
                    "channel_id": group["chat_channel"],
                    "headers": {
                        "Host": chat_host,
                        "Origin": "https://" + chat_host,
                        "Cookie": opaque.json()["cookie"],
                    },
                }
            )
            if (i + 1) % 100 == 0:
                print(
                    f"Prepared {i + 1} synthetic identities per product path",
                    flush=True,
                )
    revision = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
    ).strip()
    delta = subprocess.check_output(["git", "diff", "HEAD", "--binary"], cwd=ROOT)
    digest = hashlib.sha256(delta)
    untracked = subprocess.check_output(
        ["git", "ls-files", "--others", "--exclude-standard", "-z"], cwd=ROOT
    ).split(b"\0")
    for filename in sorted(filter(None, untracked)):
        digest.update(filename + b"\0" + (ROOT / os.fsdecode(filename)).read_bytes())

    def binary_digest(path):
        with open(path, "rb") as binary:
            return hashlib.file_digest(binary, "sha256").hexdigest()

    data = {
        "isolated": True,
        "management_url": lab.api,
        "management_headers": headers,
        "model_id": fixtures[0]["model_id"],
        "agent_id": fixtures[0]["agent_id"],
        "actors": actors,
        "metadata": {
            "label": "local authenticated llmsim capacity bench",
            "revision": revision,
            "topology": {
                "source_dirty": bool(delta or any(untracked)),
                "source_delta_sha256": digest.hexdigest(),
                "fixture_binary_sha256": binary_digest(lab.args.fixture_bin),
                "worker_binary_sha256": binary_digest(lab.args.worker_bin),
                "rustc": subprocess.check_output(
                    ["rustc", "--version"], text=True
                ).strip(),
                "node": subprocess.check_output(
                    ["node", "--version"], text=True
                ).strip(),
                "hardware": hardware(),
                "servers": 1,
                "workers": 1,
                "proxy": "Caddy HTTPS/HTTP2",
                "identity_provider": "synthetic RS256, warm key cache",
                "identity_token_lifetime_seconds": 900,
                "identity_negative_checks": ["signature", "audience", "expired"],
                "interactive_login_measured": False,
                "auth_mode": "full",
                "access_token_lifetime_seconds": 14400,
                "provider_credentials_present": False,
                "worker_slots": lab.args.worker_slots,
                "model_wait_slots": lab.args.model_waits,
                "agent_profile": "base harness + session_file_system, direct communication",
                "release_lto": False,
                "release_opt_level": 3,
                "release_codegen_units": 16,
                "binaries_built_by_coordinator": not lab.args.skip_build,
                "thread_stack_bytes": 33554432,
            },
            "limits": {
                "api_requests_per_minute": 0,
                "org_session_creates_per_minute": 100000,
                "sse_per_org": 1000,
                "sse_global": 10000,
            },
            "generator_topology": {
                "processes": lab.args.generators,
                "separate_machine": False,
            },
        },
    }
    manifest = lab.secret / "actors.json"
    private_json(manifest, data)
    return manifest


def run_case(
    lab,
    manifest,
    path,
    users,
    phase,
    duration,
    *,
    fault=False,
    restart=False,
    label="case",
):
    name = f"{path}-{users}-{phase}-{label}" + ("-recovery" if fault else "")
    directory = lab.results / name
    directory.mkdir()
    identity_before = lab.identity_metrics()
    start_file = lab.directory / (name + "-start.json")
    pids_file = lab.directory / (name + "-pids.json")
    stop_file = lab.directory / (name + "-monitor-stop")
    pids = {
        "coordinator": os.getpid(),
        **lab.native_pids,
        **{role: p.pid for role, p in lab.processes.items() if p.poll() is None},
    }
    pids_file.write_text(json.dumps(pids))
    monitor_env = {**lab.env, "CAPACITY_DATABASE_URL": lab.dburl}
    monitor = subprocess.Popen(
        [
            sys.executable,
            "-m",
            "capacity.monitor",
            "--pids",
            str(pids_file),
            "--output",
            str(directory / "resources.jsonl"),
            "--stop-file",
            str(stop_file),
            "--interval",
            "0.25" if users <= 10 else "2",
        ],
        cwd=ROOT / "scripts",
        env=monitor_env,
    )
    run_id = str(uuid.uuid4())
    shards = []
    generators = min(lab.args.generators, users)
    auth_files = []
    try:
        ack_target = (
            15000
            if fault
            else lab.args.burst_ack_p95_ms
            if phase == "burst"
            else lab.args.ack_p95_ms
        )
        for shard in range(generators):
            output = directory / f"shard-{shard}"
            ready = lab.directory / (name + f"-ready-{shard}.json")
            state = lab.secret / (name + f"-auth-{shard}.json")
            auth_files.append(state)
            command = [
                sys.executable,
                "scripts/load-test.py",
                "--capacity",
                "--manifest",
                str(manifest),
                "--path",
                path,
                "--users",
                str(users),
                "--phase",
                phase,
                "--duration",
                str(duration),
                "--interval",
                str(lab.args.interval),
                "--turn-timeout",
                str(lab.args.turn_timeout),
                "--run-id",
                run_id,
                "--shards",
                str(generators),
                "--shard-index",
                str(shard),
                "--start-file",
                str(start_file),
                "--ready-file",
                str(ready),
                "--ca-bundle",
                str(lab.cert),
                "--output",
                str(output),
                "--auth-state",
                str(state),
                "--ack-p95-ms",
                str(ack_target),
            ]
            if fault:
                command += [
                    "--fault-mode",
                    "--reconnect-at",
                    "10",
                    "--lost-ack-every",
                    "1",
                ]
            log = open(directory / f"shard-{shard}.log", "wb")
            process = subprocess.Popen(
                command, cwd=ROOT, env=lab.env, stdout=log, stderr=subprocess.STDOUT
            )
            log.close()
            shards.append((process, ready, output))
            pids[f"generator-{shard}"] = process.pid
        pids_file.write_text(json.dumps(pids))
        deadline = time.monotonic() + 600
        while not all(ready.exists() for _, ready, _ in shards):
            if time.monotonic() > deadline or any(
                p.poll() is not None for p, _, _ in shards
            ):
                raise RuntimeError("a generator failed before the synchronized start")
            time.sleep(0.25)
        epoch = time.time() + 2
        start_file.write_text(json.dumps({"epoch": epoch}))
        print(
            f"{name}: synchronized start, {users} users / {generators} generators",
            flush=True,
        )
        restarted = False
        safety_abort = None
        while any(p.poll() is None for p, _, _ in shards):
            if restart and not restarted and time.time() > epoch + 15:
                lab.restart_server()
                restarted = True
                pids["server"] = lab.processes["server"].pid
                pids_file.write_text(json.dumps(pids))
            if __import__("shutil").disk_usage(directory).free < 2 * 1024**3:
                safety_abort = "disk free below 2 GiB"
                break
            if time.time() > epoch + duration + lab.args.turn_timeout + 600:
                safety_abort = "benchmark deadline exceeded"
                break
            if any(p.poll() is not None for p in lab.processes.values()):
                safety_abort = "fixture process stopped"
                break
            time.sleep(1)
        if safety_abort:
            raise RuntimeError(safety_abort)
        for process, _, _ in shards:
            process.wait()
        report = merge([output for _, _, output in shards])
        report["case"] = name
        report["idp_refresh_successes_including_setup"] = (
            lab.identity_metrics()["refresh_successes"]
            - identity_before["refresh_successes"]
        )
        report.update(verdict(report, ack_target))
        report["ack_p95_gate_ms"] = ack_target
        stop_file.touch()
        monitor.wait(timeout=15)
        report["resources"] = summarize(
            directory / "resources.jsonl",
            report["start_epoch"],
            report["end_epoch"],
            hardware()["memory_bytes"] * 0.75,
            expected_restarts={"server"} if restarted else set(),
        )
        report["traffic_passed"] = report["passed"]
        report["passed"] = report["passed"] and report["resources"]["passed"]
        (directory / "report.json").write_text(json.dumps(report, indent=2))
        print(
            json.dumps(
                {
                    "case": name,
                    "passed": report["passed"],
                    "ack_ms": report["ack_ms"],
                    "completed": report["completed"],
                    "errors": report["errors"],
                }
            ),
            flush=True,
        )
        # Refresh state stays outside the publishable result directory.
        data = json.loads(manifest.read_text())
        updates = {}
        for state in auth_files:
            if state.exists():
                updates.update(json.loads(state.read_text()))
        for actor in data["actors"]:
            if actor["name"] in updates:
                actor["headers"] = updates[actor["name"]]
        temporary = manifest.with_suffix(".next.json")
        private_json(temporary, data)
        temporary.replace(manifest)
        return report
    finally:
        for process, _, _ in shards:
            if process.poll() is None:
                process.terminate()
        for process, _, _ in shards:
            try:
                process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        stop_file.touch()
        monitor.wait(timeout=15)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    mode = p.add_mutually_exclusive_group()
    mode.add_argument("--qualification", action="store_true")
    mode.add_argument(
        "--smoke", action="store_true", help="default: five users on each path"
    )
    p.add_argument("--users", type=int, default=1000)
    p.add_argument("--tenants", type=int, default=5)
    p.add_argument("--paths", default="console,agent-api,work-chat")
    p.add_argument("--duration", type=float, default=1800)
    p.add_argument("--interval", type=float, default=60)
    p.add_argument("--turn-timeout", type=float, default=120)
    p.add_argument("--ack-p95-ms", type=float, default=500)
    p.add_argument("--burst-ack-p95-ms", type=float, default=5000)
    p.add_argument("--generators", type=int, default=4)
    p.add_argument("--worker-slots", type=int, default=50)
    p.add_argument("--model-waits", type=int, default=200)
    p.add_argument("--port-prefix", type=int, default=284)
    p.add_argument(
        "--run-dir",
        default=str(ROOT / "target/chat-capacity" / time.strftime("%Y%m%d-%H%M%S")),
    )
    p.add_argument("--skip-build", action="store_true")
    p.add_argument("--fixture-bin", type=Path)
    p.add_argument(
        "--worker-bin", type=Path, default=ROOT / "target/release/everruns-worker"
    )
    p.add_argument("--build-jobs", type=int, default=3)
    p.add_argument("--build-log", default=str(ROOT / "target/chat-capacity-build.log"))
    a = p.parse_args()
    a.worker_bin = a.worker_bin if a.worker_bin.is_absolute() else ROOT / a.worker_bin
    if a.fixture_bin and not a.fixture_bin.is_absolute():
        a.fixture_bin = ROOT / a.fixture_bin
    if not Path(a.run_dir).is_absolute():
        a.run_dir = str(ROOT / a.run_dir)
    if (
        not 1 <= a.port_prefix <= 654
        or min(a.users, a.tenants, a.generators, a.duration, a.interval) <= 0
    ):
        p.error("invalid capacity options")
    paths = a.paths.split(",")
    if not set(paths) <= {"console", "agent-api", "work-chat"}:
        p.error("unknown product path")
    if not a.skip_build:
        build(a)
    if not a.fixture_bin or not a.fixture_bin.exists():
        p.error("provide a built fixture or omit --skip-build")
    if not a.qualification:
        a.users = min(a.users, 5)
    a.tenants = min(a.tenants, a.users)
    soft, hard = resource.getrlimit(resource.RLIMIT_NOFILE)
    desired = 8192 if hard == resource.RLIM_INFINITY else min(hard, 8192)
    if desired < 4096:
        p.error("capacity bench needs a file descriptor hard limit of at least 4096")
    resource.setrlimit(resource.RLIMIT_NOFILE, (max(soft, desired), hard))
    os.umask(0o077)
    lab = Lab(a)
    reports = []
    finished = False
    try:
        lab.start_infra()
        lab.start_identity()
        lab.start_platform()
        lab.start_host_and_proxy()
        manifest = bootstrap(lab)
        for path in paths:
            if not a.qualification:
                reports.append(
                    run_case(
                        lab,
                        manifest,
                        path,
                        a.users,
                        "burst",
                        30,
                        fault=False,
                        label="smoke",
                    )
                )
                reports.append(
                    run_case(
                        lab,
                        manifest,
                        path,
                        a.users,
                        "steady",
                        40,
                        label="stream-lifetime-smoke",
                    )
                )
                continue
            for users in sorted({min(n, a.users) for n in [100, 250, 500]}):
                reports.append(
                    run_case(lab, manifest, path, users, "steady", 60, label="ramp")
                )
            reports.append(
                run_case(
                    lab, manifest, path, a.users, "steady", a.duration, label="soak"
                )
            )
            reports.append(
                run_case(lab, manifest, path, a.users, "burst", 30, label="burst")
            )
            reports.append(
                run_case(
                    lab, manifest, path, a.users, "steady", 60, fault=True, restart=True
                )
            )
        finished = True
    finally:
        passed = finished and bool(reports) and all(r["passed"] for r in reports)
        summary = {
            "passed": passed,
            "finished": finished,
            "qualification": a.qualification,
            "qualified_1000": passed
            and a.qualification
            and a.users >= 1000
            and a.duration >= 1800,
            "cases": [
                {
                    "case": r["case"],
                    "path": r["path"],
                    "users": r["users"],
                    "phase": r["phase"],
                    "passed": r["passed"],
                    "ack_ms": r["ack_ms"],
                    "errors": r["errors"],
                }
                for r in reports
            ],
        }
        (lab.results / "summary.json").write_text(json.dumps(summary, indent=2))
        print("Saved benchmark results:", lab.results, flush=True)
        lab.stop()
    raise SystemExit(0 if passed else 1)


if __name__ == "__main__":
    main()
