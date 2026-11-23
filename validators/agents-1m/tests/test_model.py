import copy
import unittest

from checks import cluster
from checks import model as M
from tests.helpers import CFG

INPUTS = {  # the 75k gate's run 45197793040, as imported
    "node_mb_per_1k_sessions": 431.2, "node_base_mb": 61, "executor_mb_per_running_turn": 1.372,
    "turn_seconds_in_flight": 33.1, "shard_core_ms_per_turn": 51.3, "executor_core_ms_per_turn": 78.4,
    "store_puts_per_turn": 1.339,
}
PROFILE = {"shapes": {"turn_seconds": {"mean": 100.0}, "gap_minutes": {"mean": 5.0},
                      "model_calls_per_turn": {"mean": 3.0}}}


class USL(unittest.TestCase):
    def test_fit_recovers_known_contention(self):
        curve = [[n, M.usl(n, 400, 0.02, 0.0004)] for n in (1, 2, 4, 8, 16)]
        lam, sigma, kappa = M.usl_fit(curve)
        self.assertAlmostEqual(sigma, 0.02, delta=0.006)
        self.assertAlmostEqual(kappa, 0.0004, delta=0.0002)
        self.assertAlmostEqual(M.usl_peak(sigma, kappa), M.usl_peak(0.02, 0.0004), delta=8)

    def test_linear_curve_never_peaks(self):
        lam, sigma, kappa = M.usl_fit([[n, 400 * n] for n in (1, 2, 4, 8)])
        self.assertEqual(M.usl_peak(sigma, kappa), float("inf"))

    def test_needs_three_points_from_one_shard(self):
        self.assertIsNone(M.usl_fit([[1, 400], [2, 790]]))
        self.assertIsNone(M.usl_fit([[2, 790], [4, 1500], [8, 2600]]))


class Load(unittest.TestCase):
    def test_mix_only_matches_the_goal_pace(self):
        cfg = copy.deepcopy(CFG)
        cfg.goal["agents"]["coding_share"] = 0.0
        ld = M.load(cfg, INPUTS, PROFILE)
        self.assertAlmostEqual(ld["turns_per_sec"], 1_000_000 / 360)
        self.assertAlmostEqual(ld["work_per_sec"], ld["turns_per_sec"])

    def test_coding_turns_cost_more_cpu(self):
        prof = copy.deepcopy(PROFILE)
        prof["shapes"]["model_calls_per_turn"]["mean"] = 12.0
        self.assertGreater(M.load(CFG, INPUTS, prof)["work_per_sec"], M.load(CFG, INPUTS, PROFILE)["work_per_sec"])

    def test_cost_counts_store_writes_by_turn(self):
        c = M.fleet_cost(CFG, INPUTS, M.load(CFG, INPUTS, PROFILE))
        writes = 1_000_000 / 360 * 0.75 + 250_000 / 400  # mix + coding turns a second
        self.assertAlmostEqual(c["store_usd_per_hour"], writes * 1.339 * 3600 * 0.005 / 1000, places=2)


class Cluster(unittest.TestCase):
    def test_zone_loss_shifts_cpu_to_survivors(self):
        sim = cluster.simulate(CFG, INPUTS, PROFILE)
        w = sim["worst"]
        self.assertEqual(sim["shards"], 40)
        self.assertEqual(sim["node_pods"], 120)
        self.assertEqual(sim["executor_pods"] % CFG.layout["zones"], 0)
        # Memory is per replica: losing a zone does not change it.
        self.assertAlmostEqual(w["node_mem"]["zone_loss"], w["node_mem"]["peak"], places=3)
        self.assertLessEqual(w["exec_cpu"]["peak"], CFG.goal["limits"]["fleet_peak_util"] + 1e-6)
        self.assertLessEqual(w["exec_cpu"]["zone_loss"], CFG.goal["limits"]["fleet_zone_loss_util"] + 1e-6)

    def test_d8_fails_when_a_pod_is_over_its_line(self):
        big = dict(INPUTS, node_mb_per_1k_sessions=900)
        sim = cluster.simulate(CFG, big, PROFILE)
        self.assertGreater(sim["d8"], 1.0)
        self.assertIn("node_mem", sim["killed"])
        self.assertIn("shard node memory", cluster.next_limit(CFG, sim))

    def test_more_agents_more_machines(self):
        small = cluster.simulate(CFG, INPUTS, PROFILE, agents=100_000)
        big = cluster.simulate(CFG, INPUTS, PROFILE)
        self.assertLess(sum(small["machines"].values()), sum(big["machines"].values()))


if __name__ == "__main__":
    unittest.main()
