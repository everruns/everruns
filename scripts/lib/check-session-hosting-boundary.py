#!/usr/bin/env python3
"""Guard the Framework-to-host direction and the independently resolved reference."""
from pathlib import Path
import tomllib

ROOT = Path(__file__).resolve().parents[2]
workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]
reference = ROOT / "crates/durable-object"
host = tomllib.loads((reference / "Cargo.toml").read_text())
assert "workspace" in host, "hosting reference must resolve independently"
assert host["package"].get("publish") is False, "hosting reference is source-only"
assert "crates/durable-object" not in workspace["members"]
assert "crates/durable-object" in workspace["exclude"]

# Inspect aliases as well as normal dependency names, including target/dev edges.
def dependencies(manifest):
    for kind in ("dependencies", "build-dependencies", "dev-dependencies"):
        yield from manifest.get(kind, {}).items()
    for target in manifest.get("target", {}).values():
        yield from dependencies(target)

for alias, declaration in dependencies(host):
    name = declaration.get("package", alias) if isinstance(declaration, dict) else alias
    assert name not in {"everruns-core", "everruns-contracts"}, (
        f"reference imports {name}; implement hosting through everruns::hosting::spi"
    )
    if name.startswith("everruns-"):
        raise AssertionError(f"reference must depend only on the Framework facade, found {name}")

# Follow all normal/build local edges, even optional features, from Framework.
seen = set()
def visit(directory):
    directory = directory.resolve()
    if directory in seen:
        return
    seen.add(directory)
    manifest = tomllib.loads((directory / "Cargo.toml").read_text())
    name = manifest["package"]["name"]
    assert name not in {"everruns-durable-object", "everruns-serve-celld"}, (
        f"Framework depends on vendor hosting adapter {name}"
    )
    tables = [manifest, *manifest.get("target", {}).values()]
    for table in tables:
        for kind in ("dependencies", "build-dependencies"):
            for alias, declaration in table.get(kind, {}).items():
                if not isinstance(declaration, dict):
                    continue
                base = directory
                if declaration.get("workspace"):
                    declaration = workspace["dependencies"][alias]
                    base = ROOT
                if isinstance(declaration, dict) and "path" in declaration:
                    visit(base / declaration["path"])
visit(ROOT / "crates/everruns")
print("Hosting boundary: vendor-free Framework; independent reference uses only public Framework SPI.")
