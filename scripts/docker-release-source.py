#!/usr/bin/env python3
"""Resolve Docker release input before any job receives publication credentials."""

import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tomllib


def command(*args):
    return subprocess.check_output(args, text=True).strip()


def resolve(env, run=command):
    event, ref = env["EVENT_NAME"], env["GIT_REF"]
    sha = env["COMMIT_SHA"]
    source = sha
    tag = ""
    latest = False
    # amd64 only: PR validation, and a main snapshot that asks for it (the
    # nightly v2 workflow checks run on amd64 runners, at half the build).
    amd64 = event == "pull_request"
    if event == "pull_request":
        sha = env["PR_HEAD_SHA"]
    elif event == "workflow_dispatch" and not env.get("INPUT_TAG"):
        # Main snapshot: a hand dispatch from main with no tag publishes the
        # dispatched commit under its SHA tags only (never `latest` or a
        # version), so `everruns k8s up --image-tag <sha>` can run current main
        # between releases. Decision: on demand, not per push, see
        # knowledge/project/release-process.md#trigger-rationale.
        if ref != "refs/heads/main":
            raise ValueError("a snapshot without a tag must be dispatched from main")
        amd64 = env.get("INPUT_AMD64_ONLY") == "true"
    else:
        tag = env["INPUT_TAG"] if event == "workflow_dispatch" else env["REF_NAME"]
        if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+([.-][0-9A-Za-z]+)*)?", tag):
            raise ValueError("release input must be a semantic version tag")
        recovery = event == "workflow_dispatch" and ref == "refs/heads/main"
        if not recovery and (ref != f"refs/tags/{tag}" or env["REF_NAME"] != tag):
            raise ValueError("dispatch must use trusted main or the matching version tag")
        if event not in ("push", "workflow_dispatch"):
            raise ValueError("unsupported release event")
        if recovery:
            repo = env["GITHUB_REPOSITORY"]
            release = json.loads(run("gh", "api", f"repos/{repo}/releases/tags/{tag}"))
            if release["draft"] or not release["published_at"] or release["tag_name"] != tag:
                raise ValueError("recovery requires an existing published release")
            # Fetch the immutable tag, not the release API's target_commitish (often 'main').
            run("git", "fetch", "--no-tags", "origin", f"refs/tags/{tag}")
            source = sha = run("git", "rev-parse", "FETCH_HEAD^{commit}")
            manifest = tomllib.loads(run("git", "show", f"{sha}:Cargo.toml"))
            if manifest["workspace"]["package"]["version"] != tag[1:]:
                raise ValueError("release tag and workspace version disagree")
            current = json.loads(run("gh", "api", f"repos/{repo}/releases/latest"))
            latest = current["tag_name"] == tag
        else:
            latest = True
    if not re.fullmatch(r"[0-9a-f]{40}", sha) or not re.fullmatch(r"[0-9a-f]{40}", source):
        raise ValueError("source must resolve to a full commit SHA")
    return {
        "ref": source,
        "sha": sha,
        "sha_short": sha[:7],
        "tag": tag,
        "is_release": str(bool(tag)).lower(),
        "is_latest": str(latest).lower(),
        "platforms": "linux/amd64" if amd64 else "linux/amd64,linux/arm64",
        # Architectures whose release binaries are compiled natively (one runner
        # per architecture) before the images package them.
        "arches": json.dumps(["amd64"] if amd64 else ["amd64", "arm64"]),
    }


if __name__ == "__main__":
    try:
        outputs = resolve(os.environ)
        with Path(os.environ["GITHUB_OUTPUT"]).open("a") as output:
            for key, value in outputs.items():
                print(f"{key}={value}", file=output)
        print(f"Docker source: {outputs['ref']}; release: {outputs['tag'] or 'PR validation'}")
    except (ValueError, KeyError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
