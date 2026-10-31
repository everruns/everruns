"""Credential-free safety checks for the walkthrough client."""

import os
from pathlib import Path
import runpy
import tempfile
import unittest
from unittest.mock import patch


Demo = runpy.run_path(str(Path(__file__).resolve().parents[1] / "demo.py"))["Demo"]


class DisposableStoreTest(unittest.TestCase):
    def test_start_does_not_inherit_a_remote_store(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "binary"
            binary.touch()
            with (
                patch.dict(os.environ, {"SERVE_STORE": "s3://must-not-use/shared"}),
                patch("tempfile.mkdtemp", return_value=directory),
                patch("subprocess.Popen") as launch,
                patch.object(Demo, "request", return_value={}),
            ):
                launch.return_value.poll.return_value = None
                demo = Demo(binary)
                try:
                    demo.start()
                    env = launch.call_args.kwargs["env"]
                    # Do not print the inherited environment if this regresses.
                    self.assertFalse("SERVE_STORE" in env)
                    self.assertEqual(env["SERVE_DATA_DIR"], str(Path(directory) / "data"))
                    self.assertEqual(env["SERVE_API_KEYS"], demo.token)
                    self.assertEqual(os.environ["SERVE_STORE"], "s3://must-not-use/shared")
                finally:
                    demo.stop()


if __name__ == "__main__":
    unittest.main()
