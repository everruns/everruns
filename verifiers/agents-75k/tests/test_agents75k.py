"""The 75k verifier: its goal and points, the reliability and durability
adapters it adds, and the backtest on the recorded 75k runs at full (L4)
certainty with the repeat-pass streak (R10)."""

import os
import unittest

from checks import cloud, fleettests, formal, local, registry, reliability
from engine import cli
from engine import config as C
from engine import score as S

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
CFG = C.load(ROOT)
CFG_1M = C.load(os.path.join(os.path.dirname(ROOT), "agents-1m"))
RUNS = os.path.join(os.path.dirname(ROOT), "agents-1m", "tests", "fixtures", "runs")
ORDER = ["43774871232", "43803384547", "43827766384", "44355318472", "44396952768", "45179434646", "45197793040"]
NOW = S.parse_time("2026-11-23T20:00:00Z")


def fresh():
    return S.Freshness(changed=lambda commit, paths: False, now=NOW)


class Goal(unittest.TestCase):
    def test_points_and_weights(self):
        self.assertEqual(len(CFG.points), 58)
        self.assertEqual(CFG.total_weight, 157)
        ids = {p.id for p in CFG.points}
        self.assertTrue({f"R{i}" for i in range(1, 11)} <= ids)
        self.assertTrue({f"U{i}" for i in range(1, 14)} <= ids)
        self.assertFalse({"D1", "D4", "D6"} & ids)  # scaling past 75k is the 1M verifier's

    def test_no_cloud_slice_and_a_near_full_run_counts(self):
        self.assertNotIn("L3", CFG.levels)
        self.assertEqual(CFG.hooks.full_size(CFG), 75_000 * 0.95)
        self.assertEqual(CFG.derived()["shards"], 3)
        self.assertEqual(CFG.title, "75k agents verifier")

    def test_targets_evaluate(self):
        self.assertEqual(CFG.target(CFG.point("U7")), 60)
        self.assertEqual(CFG.target(CFG.point("R10")), 3)


class Registry(unittest.TestCase):
    def names(self, cfg):
        return {c.name: c for c in registry(cfg)}

    def test_each_verifier_gets_only_its_checks(self):
        mine, big = self.names(CFG), self.names(CFG_1M)
        for name in ("tla-models", "prove-leader-failover", "chaos", "soak"):
            self.assertIn(name, mine)
            self.assertNotIn(name, big)
        self.assertNotIn("cloud-slice", mine)
        self.assertIn("cloud-slice", big)
        self.assertNotIn("scale", mine)  # D1 only
        self.assertEqual(mine["full"].cost_usd, 40)
        self.assertIn("R10", mine["full"].points)
        self.assertNotIn("R10", big["full"].points)

    def test_fleet_tests_run_only_what_points_need(self):
        plan = fleettests.wanted(CFG)
        self.assertIn("snapshot_memory", plan["everruns-agentd"][0])
        plan_1m = fleettests.wanted(CFG_1M)
        self.assertEqual(set(plan_1m["everruns-agentd"][0]), {"raft_simulation"})
        self.assertFalse(plan_1m["everruns-agentd-testkit"][1])  # no U4-U6 for 1M


class Adapters(unittest.TestCase):
    def test_per_test_points(self):
        text = ("     Running tests/recovery.rs (x)\n"
                "test a_shard_that_loses_two_zones_comes_back_from_the_store_and_the_survivor ... ok\n"
                "test two_zones_lost_while_the_survivor_lagged_still_keeps_every_started_call ... FAILED\n"
                "test a_point_in_time_restore_drops_what_came_after ... ok\n"
                "test result: FAILED. 2 passed; 1 failed\n")
        got = fleettests.parse_tests(text, ["recovery"])
        out = cloud.Output()
        fleettests.test_entries(got, fleettests.BY_TEST, out, "cmd")
        v = {e["point"]: e["value"] for e in out.entries}
        self.assertEqual(v, {"U4": False, "U5": True, "U6": False})  # U6 did not run: fails, never skipped

    def test_drill_durability_numbers(self):
        res = {"sessions": 75000, "shards": 2, "inflight": 3000, "rebuilt": {"ok": True}, "rebuild_s": 3.7,
               "kill_to_leader_s": 7.4, "second_messages_lost": 18, "ship_lag_s": {"p99": 1.5},
               "ship_first_s": {"p50": 0.01, "p99": 0.1}, "check": {"first": {"ok": True}, "second": {"ok": True}}}
        v = {e["point"]: e["value"] for e in local.drill(res).entries}
        self.assertEqual((v["U7"], v["U8"], v["U9"], v["U10"]), (3.7, 7.4, 0.1, 0.006))

    def test_proof_verdicts(self):
        self.assertEqual(reliability.verdict({"ok": True, "errors": 0}), (True, 0))
        self.assertEqual(reliability.verdict({"load_errors": 2, "check": {"ok": True}}), (False, 2))
        self.assertEqual(reliability.verdict({"load_errors": 0, "check": {"a": {"ok": True}, "b": {"ok": False}}}), (False, 0))
        out = reliability.leader_failover({"ok": True, "election_secs": 1.2, "model_calls_started": 2,
                                           "expected_model_calls": 2, "restarted_leader_caught_up": True})
        self.assertTrue(out.entries[0]["value"])

    def test_every_proof_reads_a_script_that_exists(self):
        for ch in reliability.CHECKS:
            script = ch.command.split()[0]
            self.assertTrue(os.path.exists(os.path.join(C.REPO, script)), script)
        self.assertTrue(os.path.exists(formal.SCRIPT))

    def test_tla_output(self):
        good = "ok   Moves (pass)\nok   MovesUnfenced (fail)\n"
        self.assertTrue(formal.parse(good, 0).entries[0]["value"])
        bad = good + "FAIL GrantsExpiryReleases: expected fail, got pass\n"
        e = formal.parse(bad, 1).entries[0]
        self.assertFalse(e["value"])
        self.assertIn("GrantsExpiryReleases", e["note"])
        self.assertFalse(formal.parse("curl: (22) 403\n", 1).entries[0]["value"])


class Backtest(unittest.TestCase):
    def test_full_certainty_and_the_streak(self):
        runs, streaks = [], []
        for i, rid in enumerate(ORDER):
            data = cloud.read_run(os.path.join(RUNS, rid))
            out = cloud.import_run(data, rid, "L4", 75_000)
            streak = cli.gate_streak(CFG, runs, out.entries)
            streaks.append(streak)
            runs.append({"run": rid, "at": f"2026-11-{10 + i}T00:00:00Z", "commit": "x", "imported": rid,
                         "fingerprint": CFG.fingerprint, "levels": ["L4"], "agents": out.agents,
                         "density": out.density, "entries": out.entries, "invalid": []})
        self.assertEqual(streaks, [0, 0, 0, 0, 1, 0, 1])
        sts = S.states(CFG, runs, fresh())
        self.assertFalse(any(st.gate_failed for st in sts.values()))  # newest gate result wins
        self.assertEqual(sts["A1"].cells["L4"].c, 1.0)  # 72.5k of 75k counts as full
        self.assertEqual(CFG.hooks.proven_agents(CFG, runs), 72_537)


if __name__ == "__main__":
    unittest.main()


class GateImport(unittest.TestCase):
    """Gate runs go into the journal in order, each with the commit its
    agentd image was built from."""

    DIGEST = "sha256:" + "a" * 64

    def test_image_commit_reads_the_revision_label_through_an_index(self):
        amd, arm = "sha256:" + "1" * 64, "sha256:" + "2" * 64
        answers = {
            "token": {"token": "t"},
            f"manifests/{self.DIGEST}": {"manifests": [
                {"digest": arm, "platform": {"architecture": "arm64"}},
                {"digest": amd, "platform": {"architecture": "amd64"}}]},
            f"manifests/{amd}": {"config": {"digest": "sha256:cfg"}},
            "blobs/sha256:cfg": {"config": {"Labels": {"org.opencontainers.image.revision": "c78b62d7"}}},
        }
        seen = []

        def get(url, token=None, accept=None):
            seen.append(url)
            return next(v for k, v in answers.items() if url.endswith(k) or (k == "token" and "/token?" in url))

        self.assertEqual(cloud.image_commit(self.DIGEST, get), "c78b62d7")
        self.assertFalse(any(arm in u for u in seen))

    def test_run_image_finds_the_agentd_digest(self):
        import json
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            self.assertIsNone(cloud.run_image(d))
            with open(os.path.join(d, "pods.json"), "w") as f:
                json.dump({"items": [{"spec": {"containers": [
                    {"image": "timberio/vector:0.50"}, {"image": f"ghcr.io/everruns/agentd@{self.DIGEST}"}]}}]}, f)
            self.assertEqual(cloud.run_image(d), self.DIGEST)

    def test_only_gate_sized_runs_are_gate_attempts(self):
        self.assertTrue(cloud.is_gate({"summary": {"peak_sessions": 79_197, "sessions": 72_027}}, 75_000))
        self.assertFalse(cloud.is_gate({"summary": {"sessions": 12_000}}, 75_000))
        self.assertFalse(cloud.is_gate({"summary": None}, 75_000))

    def test_next_gate_run_takes_the_oldest_new_gate_run(self):
        import tempfile
        from unittest import mock
        V = type(CFG.hooks).command.__globals__  # this verifier's verifier.py, as config loaded it
        runs = [{"imported": "45197793040"}, {"imported": "run-45313187555"}, {"source": "verify"}]
        self.assertEqual(V["imported_ids"](runs), {45197793040, 45313187555})
        sizes = {1: 10_000, 2: 75_000, 3: 75_000}
        with tempfile.TemporaryDirectory() as d, \
                mock.patch.object(cloud, "gate_runs", return_value=[1, 2, 3]) as listed, \
                mock.patch.object(cloud, "fetch", side_effect=lambda rid, dest: str(rid)), \
                mock.patch.object(cloud, "read_run", side_effect=lambda p: {"summary": {"sessions": sizes[int(p)]}}):
            rid, path, data = V["next_gate_run"](CFG, {"runs": runs, "evidence": d})
        listed.assert_called_once_with(45313187555)
        self.assertEqual(rid, "2")
