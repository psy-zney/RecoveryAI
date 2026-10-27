"""Test-only local process transport; never represents SSH authentication tests."""
import os
import shlex
import subprocess
import sys

command = shlex.split(sys.argv[-1])
assert command[:2] == ["python3", "-"]
result = subprocess.run([sys.executable, *command[1:]], input=sys.stdin.buffer.read(), capture_output=True)
if os.environ.get("RESC_TEST_DROP_STREAM") and "stream" in command:
    sys.stdout.buffer.write(result.stdout[:1024])
    sys.exit(1)
sys.stdout.buffer.write(result.stdout)
sys.stderr.buffer.write(result.stderr)
sys.exit(result.returncode)
