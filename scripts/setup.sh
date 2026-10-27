#!/usr/bin/env bash
# Repository setup: no source/lockfile modifications or unverified downloads.
set -euo pipefail
cd "$(dirname "$0")/.."
for tool in cargo rustc node python cc; do
  command -v "$tool" >/dev/null || { echo "Missing $tool; see docs/recovery-guide.vi.md" >&2; exit 1; }
done
python -c 'import sys; assert sys.version_info >= (3,11), "Python >=3.11 is required"'
node -e 'if (Number(process.versions.node.split(".")[0]) < 18) process.exit(1)'
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
cargo fetch --locked
cargo build --release --workspace --locked
python scripts/recovery.py doctor
node bin/resc-mcp.js --version
