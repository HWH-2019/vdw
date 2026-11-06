#!/usr/bin/env bash
# Development/release build only. Installed VDW never invokes this script or Node.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${ROOT}/vdw-dashboard"
if ! command -v node >/dev/null || ! command -v npm >/dev/null; then
  echo "Building dashboard sources requires Node >=20.19 and npm. Installed release binaries require neither." >&2
  exit 1
fi
npm ci --no-audit --no-fund
npm run build
npm run verify
