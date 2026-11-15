#!/usr/bin/env bash
# Type-checks the native mod against a local Horizon checkout, without the launcher.
#   tools/check_native.sh [path to horizon checkout]
# The bundle itself may hold no Cargo.lock, so this works on a copy under target/.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
horizon="$(cd "${1:-$root/../horizon-src}" && pwd)"
# Cargo wants a Windows path under Git Bash.
if command -v cygpath >/dev/null 2>&1; then horizon="$(cygpath -m "$horizon")"; fi
work="$root/target/native-check"
rm -rf "$work/native"
mkdir -p "$work"
cp -r "$root/native" "$work/native"
find "$work/native" -name Cargo.lock -delete
python "$root/tools/gen_modules.py" "$work/native"
# Horizon's own lock, so dependencies resolve to the versions the game is built with.
cp "$horizon/Cargo.lock" "$work/native/api/Cargo.lock"
cd "$work/native/api"
# A crate outside Horizon's workspace doesn't inherit its [patch.crates-io] (patched wgpu-hal, naga
# and libmimalloc-sys), and the game doesn't compile against the unpatched ones: repeat them here.
CARGO_TARGET_DIR="$root/target/native-check/target" cargo check --tests   --config "patch.\"https://github.com/horizon-rust/horizon\".horizon-client.path=\"$horizon/crates/horizon-client\""   --config "patch.crates-io.wgpu-hal.path=\"$horizon/third_party/wgpu-hal\""   --config "patch.crates-io.naga.path=\"$horizon/third_party/naga\""   --config "patch.crates-io.libmimalloc-sys.path=\"$horizon/third_party/libmimalloc-sys\""
