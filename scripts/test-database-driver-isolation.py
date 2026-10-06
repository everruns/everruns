#!/usr/bin/env python3
"""Regression probes for database ownership, aliases, features, and constructors."""

import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    "database_guard", Path(__file__).parent / "lib/check-database-driver-isolation.py"
)
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


def package(name, dependency="sqlx", kind=None, optional=False, rename=None):
    return {
        "name": name,
        "dependencies": [{"name": dependency, "kind": kind, "optional": optional, "rename": rename}],
    }


class DatabaseOwnership(unittest.TestCase):
    def test_normal_build_optional_and_renamed_driver_edges_are_rejected(self):
        for kind in [None, "build"]:
            for optional in [False, True]:
                for driver in ["sqlx", "rusqlite", "tokio-postgres"]:
                    with self.subTest(kind=kind, optional=optional, driver=driver):
                        self.assertTrue(guard.dependency_violations([
                            package("everruns-worker", driver, kind, optional, "hidden_backend")
                        ]))

    def test_only_owning_crates_and_dev_dependencies_are_allowed(self):
        for name in ["everruns-server", "everruns-durable", "everruns"]:
            self.assertFalse(guard.dependency_violations([package(name, "rusqlite")]))
        self.assertFalse(guard.dependency_violations([package("everruns-core", kind="dev")]))

    def test_typed_id_codec_exception_is_narrow(self):
        self.assertFalse(guard.dependency_violations([
            package("everruns-contracts", optional=True)
        ]))
        for dependency, kind, optional in [("sqlx", None, False), ("sqlx", "build", True), ("rusqlite", None, True)]:
            self.assertTrue(guard.dependency_violations([
                package("everruns-contracts", dependency, kind, optional)
            ]))
        self.assertTrue(guard.dependency_violations([
            package("everruns-contracts", optional=True, rename="hidden_backend")
        ]))

    def test_codec_exception_does_not_allow_connections_or_pool_builders(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "src").mkdir()
            source = root / "src/lib.rs"
            entry = {"name": "everruns-contracts", "manifest_path": str(root / "Cargo.toml")}
            source.write_text("// Connection::open(ignored)\nimpl Codec {}\n")
            self.assertFalse(guard.constructor_violations([entry]))
            for code in [
                "Connection::open(path);",
                "QueryConnection::open(path);",
                "PgPoolOptions::new().connect(url);",
                "Connection::\nopen_in_memory();",
                "use sqlx::PgPool as Db; Db::connect(url);",
                "use sqlx::{Pool as Db, Postgres}; Db::<Postgres>::connect(url);",
            ]:
                source.write_text(code)
                self.assertTrue(guard.constructor_violations([entry]))
            for owner in ["everruns-durable", "everruns"]:
                entry["name"] = owner
                self.assertFalse(guard.constructor_violations([entry]))


    def test_forwarded_query_types_allow_callbacks_but_not_aliased_construction(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "src").mkdir()
            source = root / "src/lib.rs"
            entry = {"name": "everruns-serve", "manifest_path": str(root / "Cargo.toml")}
            for code in [
                "use everruns::sqlite::QueryConnection as Db; Db::open(path);",
                "use everruns::sqlite::{Connection as Db, Row}; Db::open_in_memory();",
                "type Db = everruns::sqlite::QueryConnection; Db::open(path);",
                "type First = everruns::sqlite::Connection; type Db = First; Db::open_with_flags(path, flags);",
                "use everruns_durable::PostgresPool as Db; Db::connect_lazy(url);",
            ]:
                with self.subTest(code=code):
                    source.write_text(code)
                    self.assertTrue(guard.constructor_violations([entry]))
            source.write_text(
                "use everruns::sqlite::QueryConnection as Db; "
                "type QueryDb = Db; fn read(conn: &QueryDb) { conn.execute(sql, params); }"
            )
            self.assertFalse(guard.constructor_violations([entry]))


if __name__ == "__main__":
    unittest.main()
