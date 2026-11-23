import json
import os
import tempfile
import unittest

from checks import realism
from workload import capture, generate

SECRET = "TOP-SECRET-PROMPT-TEXT"


def transcript(path):
    rows = [
        {"type": "user", "timestamp": "2026-11-20T10:00:00Z", "message": {"role": "user", "content": SECRET}},
        {"type": "assistant", "timestamp": "2026-11-20T10:00:05Z", "message": {
            "id": "m1", "usage": {"input_tokens": 10, "cache_read_input_tokens": 1000, "cache_creation_input_tokens": 0, "output_tokens": 50},
            "content": [{"type": "tool_use", "name": "Bash", "input": {"command": SECRET}}]}},
        {"type": "user", "timestamp": "2026-11-20T10:00:09Z", "message": {"content": [
            {"type": "tool_result", "content": "x" * 2048}]}},
        {"type": "assistant", "timestamp": "2026-11-20T10:00:20Z", "message": {
            "id": "m2", "usage": {"input_tokens": 10, "cache_read_input_tokens": 3000, "output_tokens": 80},
            "content": [{"type": "text", "text": SECRET}]}},
        {"type": "user", "timestamp": "2026-11-20T10:30:20Z", "message": {"role": "user", "content": "next " + SECRET}},
        {"type": "assistant", "timestamp": "2026-11-20T10:30:30Z", "message": {
            "id": "m3", "usage": {"input_tokens": 10, "cache_read_input_tokens": 4000, "output_tokens": 20},
            "content": [{"type": "tool_use", "name": "mcp__github__get_pr", "input": {}}]}},
    ]
    with open(path, "w") as f:
        for r in rows:
            f.write(json.dumps(r) + "\n")


class Capture(unittest.TestCase):
    def test_numbers_only(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "s.jsonl")
            transcript(p)
            turns, extra = capture.transcript_turns(p)
            prof = capture.profile([(turns, extra)])
            text = json.dumps(prof)
        self.assertNotIn(SECRET, text)
        self.assertNotIn("Bash", text)
        self.assertNotIn("github", text)
        self.assertEqual([t["model_calls"] for t in turns], [2, 1])
        self.assertAlmostEqual(turns[0]["seconds"], 20)
        self.assertAlmostEqual(turns[0]["gap_minutes"], 30)
        self.assertEqual(turns[0]["context_tokens"], [1010, 3010])
        self.assertAlmostEqual(turns[0]["tool_kb"][0], 2.0)
        self.assertEqual(prof["tool_kinds"], {"mcp": 0.5, "shell": 0.5})

    def test_list_events_dump(self):
        here = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
        turns, _ = capture.event_turns(os.path.join(os.path.dirname(here), "lib", "workload", "sources", "bashkit-issue.events.json"))
        self.assertEqual([t["model_calls"] for t in turns], [5, 2, 6, 2])
        self.assertAlmostEqual(turns[0]["gap_minutes"], (35 * 60 - 3 - 10.146) / 60, places=1)


class Generate(unittest.TestCase):
    def test_seq_bytes(self):
        self.assertEqual(generate.seq_bytes(9), 18)
        self.assertEqual(generate.seq_bytes(10), 21)
        self.assertGreaterEqual(generate.seq_bytes(generate.seq_for_kb(7.4)), 7.4 * 1024)

    def test_committed_scenarios_are_current_and_realistic(self):
        with open(generate.PROFILE) as f:
            prof = json.load(f)
        with open(generate.OUT) as f:
            have = f.read()
        self.assertEqual(have, generate.render(prof), "run workload/generate.py")
        worst, detail = realism.distance(prof, json.loads(have))
        self.assertLessEqual(worst, 0.10, detail)

    def test_realism_catches_a_load_off_profile(self):
        with open(generate.PROFILE) as f:
            prof = json.load(f)
        scens = generate.scenarios(prof)
        for s in scens:
            s["steps"] = s["steps"][:1] + s["steps"][-1:]  # every turn cut to two model calls
        worst, _ = realism.distance(prof, scens)
        self.assertGreater(worst, 0.10)

    def test_deterministic(self):
        with open(generate.PROFILE) as f:
            prof = json.load(f)
        self.assertEqual(generate.render(prof), generate.render(prof))


if __name__ == "__main__":
    unittest.main()
