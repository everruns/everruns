#!/usr/bin/env python3
"""Exercise control-plane isolation against real standalone Cargo workspaces."""
from pathlib import Path
import subprocess
import tempfile
import unittest

GUARD = Path(__file__).with_name('check_control_plane_records.py')


class RecordIsolation(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='everruns-record-guard-')
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / 'Cargo.toml').write_text(
            '[workspace]\nmembers = ["crates/server", "crates/library", "crates/worker"]\nresolver = "3"\n'
        )
        for name, publish in [('server', False), ('library', True), ('worker', False)]:
            crate = self.root / 'crates' / name
            (crate / 'src').mkdir(parents=True)
            (crate / 'Cargo.toml').write_text(
                f'[package]\nname = "everruns-{name}"\nversion = "0.1.0"\nedition = "2024"\n'
                + ('' if publish else 'publish = false\n')
            )
            (crate / 'src/lib.rs').write_text('')
        records = self.root / 'crates/server/src/records'
        records.mkdir()
        (records / 'agent.rs').write_text('pub struct Agent;\npub enum AgentStatus { Active }\n')

    def guard(self):
        return subprocess.run(
            ['python3', str(GUARD), '--root', str(self.root)], capture_output=True, text=True,
        )

    def test_server_record_is_allowed(self):
        self.assertEqual(self.guard().returncode, 0)

    def test_published_alias_cannot_reintroduce_record(self):
        (self.root / 'crates/library/src/lib.rs').write_text('pub type Agent = serde_json::Value;\n')
        result = self.guard()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('control-plane Agent must be server-owned', result.stdout)

    def test_private_worker_record_cannot_hide_from_guard(self):
        (self.root / 'crates/worker/src/lib.rs').write_text('struct Agent { created_at: String }\n')
        result = self.guard()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('control-plane Agent must be server-owned', result.stdout)

    def test_published_server_dependency_is_rejected(self):
        manifest = self.root / 'crates/library/Cargo.toml'
        manifest.write_text(manifest.read_text() + '[dependencies]\neverruns-server = { path = "../server" }\n')
        result = self.guard()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('published crate depends on the server', result.stdout)

    def test_private_adapter_cannot_import_record(self):
        (self.root / 'crates/worker/src/lib.rs').write_text('use everruns_server::records::Agent;\n')
        result = self.guard()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('references server records', result.stdout)


if __name__ == '__main__':
    unittest.main()
