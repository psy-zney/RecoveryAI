#!/usr/bin/env bash
# Check an existing Linux Live environment; does not modify disks or services.
set -euo pipefail
test "$(uname -s)" = Linux || { echo 'Run this on Linux Live.' >&2; exit 1; }
missing=0
for tool in python3 lsblk ddrescue ssh; do
  if ! command -v "$tool" >/dev/null; then
    echo "Missing: $tool" >&2
    missing=1
  fi
done
if test "$missing" = 1; then
  echo 'Debian/Ubuntu Live: sudo apt-get update && sudo apt-get install python3 util-linux gddrescue openssh-client' >&2
  exit 1
fi
python3 -c 'import sys; assert sys.version_info >= (3,11), "Python >=3.11 required"'
ddrescue --version
echo 'Ready for rescue.py devices / acquire. SSH server setup is separate; see docs/two-computer-rescue.vi.md.'
