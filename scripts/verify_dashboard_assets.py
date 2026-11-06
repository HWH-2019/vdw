#!/usr/bin/env python3
"""Verify checked-in dashboard resources without Node/npm or third-party modules."""
import hashlib
import json
from pathlib import Path


def verify(root=None):
    root = Path(root or Path(__file__).resolve().parents[1])
    assets = root / "vdw" / "dashboard-assets"
    manifest = json.loads((assets / "manifest.json").read_text())
    for base, key in [(assets, "files"), (root / "vdw-dashboard", "sources")]:
        expected = manifest[key]
        for name, digest in expected.items():
            path = base / name
            if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != digest:
                raise SystemExit(f"Stale dashboard file: {path}. Run scripts/build_dashboard.sh before releasing.")
    sources = root / "vdw-dashboard"
    actual = {str(p.relative_to(sources)).replace("\\", "/") for directory in ("src", "public")
              for p in (sources / directory).rglob("*") if p.is_file()}
    listed = {p for p in manifest["sources"] if p.startswith(("src/", "public/"))}
    if actual != listed:
        raise SystemExit("Dashboard source inventory changed; rebuild with scripts/build_dashboard.sh")
    print("Dashboard source and asset hashes verified (no Node/npm required).")


if __name__ == "__main__":
    verify()
