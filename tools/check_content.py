"""Check the staged content against the new loader's schema in seconds, instead of finding out from a 10 to 30
minute native-build refusal.

    python tools/stage_package.py && python tools/check_content.py

Runs the checkout's `cargo-horizon-mod horizon-mod check` in target/package (the registry's checks, the same reader
the launcher uses; about 6 seconds).
"""
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
package = ROOT / "target" / "package"
tool = None
for parent in ROOT.parents:
    candidate = parent / "horizon-latest" / "target" / "debug" / "cargo-horizon-mod.exe"
    if candidate.is_file():
        tool = candidate
        break
if tool is None:
    sys.exit("cargo-horizon-mod was not found beside the repository (horizon-latest/target/debug)")

# `check` reads the project in the current folder: the staged package.
result = subprocess.run([str(tool), "horizon-mod", "check"], cwd=package, capture_output=True, text=True)
print((result.stdout + result.stderr).strip()[:6000])
sys.exit(result.returncode)
