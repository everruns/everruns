#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PY'
import importlib.util
import json
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("source", "scripts/docker-release-source.py")
source = importlib.util.module_from_spec(spec)
spec.loader.exec_module(source)
MAIN, RELEASE, PR = "a" * 40, "b" * 40, "c" * 40

class ReleaseSourceTests(unittest.TestCase):
    def env(self, **changes):
        return dict(EVENT_NAME="workflow_dispatch", GIT_REF="refs/heads/main",
                    REF_NAME="main", INPUT_TAG="v0.46.0", COMMIT_SHA=MAIN,
                    PR_HEAD_SHA=PR, GITHUB_REPOSITORY="everruns/everruns", **changes)

    def run_api(self, latest="v0.46.0", draft=False, version="0.46.0", published="today"):
        def run(*args):
            if args[0] == "gh":
                return json.dumps(dict(tag_name=latest if args[-1].endswith("/latest") else "v0.46.0",
                                       draft=draft, published_at=published))
            if args[1] == "fetch":
                return ""
            if args[1] == "rev-parse":
                return RELEASE
            return f'[workspace.package]\nversion = "{version}"'
        return run

    def test_recovery_uses_tag_commit_not_workflow_main(self):
        result = source.resolve(self.env(), self.run_api())
        self.assertEqual((result["ref"], result["sha"], result["tag"]), (RELEASE, RELEASE, "v0.46.0"))
        self.assertEqual(result["is_latest"], "true")

    def test_older_release_does_not_overwrite_latest(self):
        self.assertEqual(source.resolve(self.env(), self.run_api(latest="v0.47.0"))["is_latest"], "false")

    def test_unpublished_and_misversioned_release_rejected(self):
        for options in (dict(draft=True), dict(published=None), dict(version="0.47.0")):
            with self.subTest(options=options), self.assertRaises(ValueError):
                source.resolve(self.env(), self.run_api(**options))

    def test_untrusted_refs_mismatched_tags_and_injection_rejected_before_fetch(self):
        for changes in (dict(GIT_REF="refs/heads/feature", REF_NAME="feature"),
                        dict(GIT_REF="refs/tags/v0.45.0", REF_NAME="v0.45.0"),
                        dict(INPUT_TAG="v0.46.0;$(echo injected)"), dict(INPUT_TAG="../main"),
                        dict(INPUT_TAG="v0.46.0\nsha=evil"), dict(INPUT_TAG="main")):
            env = self.env()
            env.update(changes)
            def no_network(*args):
                self.fail("invalid release reached network/git")
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                source.resolve(env, no_network)

    def test_matching_tag_dispatch_and_push_still_publish(self):
        for event in ("push", "workflow_dispatch"):
            env = self.env()
            env.update(EVENT_NAME=event, GIT_REF="refs/tags/v0.46.0", REF_NAME="v0.46.0", COMMIT_SHA=RELEASE)
            result = source.resolve(env)
            self.assertEqual((result["ref"], result["is_release"], result["is_latest"]), (RELEASE, "true", "true"))

    def test_pr_retains_merge_checkout_and_head_sha_tags(self):
        env = self.env()
        env.update(EVENT_NAME="pull_request", GIT_REF="refs/pull/1/merge")
        result = source.resolve(env)
        self.assertEqual((result["ref"], result["sha"], result["tag"], result["is_latest"], result["platforms"]),
                         (MAIN, PR, "", "false", "linux/amd64"))

unittest.main()
PY
