#!/usr/bin/env bash
# Tests and type-checks the native mod as one agent sees it: its own modules plus everything
# finished, without the modules other agents are still writing.
#   tools/check_module.sh <owner> [path to horizon checkout]
# tools/wip.txt lists who is writing what: `<owner>: <module> <module> ...`. A module name
# covers native/api/src/machines/<name>.rs and <name>_*.rs, and the same under
# native/kinetics/src/rules/. Other owners' files are left out of a private copy under
# target/check-<owner>/, the module lists are regenerated there, and then it runs
#   cargo test   on native/kinetics (the rules)
#   cargo check  on native/api against Horizon (the hooks)
# With an owner of `all`, nothing is left out: the whole tree as it stands.
set -euo pipefail
owner="${1:?usage: tools/check_module.sh <owner> [horizon checkout]}"
root="$(cd "$(dirname "$0")/.." && pwd)"
horizon="$(cd "${2:-$root/../horizon-src}" && pwd)"
if command -v cygpath >/dev/null 2>&1; then horizon="$(cygpath -m "$horizon")"; fi
work="$root/target/check-$owner"
rm -rf "$work/native"
mkdir -p "$work"
cp -r "$root/native" "$work/native"
find "$work/native" -name Cargo.lock -delete
if [ "$owner" != all ] && [ -f "$root/tools/wip.txt" ]; then
  while IFS= read -r line; do
    case "$line" in ''|'#'*) continue;; esac
    who="${line%%:*}"
    [ "$who" = "$owner" ] && continue
    for name in ${line#*:}; do
      rm -f "$work/native/api/src/machines/$name.rs" "$work/native/api/src/machines/${name}"_*.rs \
            "$work/native/kinetics/src/rules/$name.rs" "$work/native/kinetics/src/rules/${name}"_*.rs
    done
  done < "$root/tools/wip.txt"
fi
python "$root/tools/gen_modules.py" "$work/native"
# One target directory for every owner: Horizon's client library is compiled once.
export CARGO_TARGET_DIR="$root/target/native-check/target"
echo "== cargo test: native/kinetics =="
(cd "$work/native/kinetics" && cargo test 2>&1 | grep -v '^\s*Compiling\|^\s*Blocking' | tail -n 40)
rm -f "$work/native/kinetics/Cargo.lock"
# Horizon's own lock, so dependencies resolve to the versions the game is built with.
cp "$horizon/Cargo.lock" "$work/native/api/Cargo.lock"
cd "$work/native/api"
echo "== cargo check: native/api =="
# A crate outside Horizon's workspace doesn't inherit its [patch.crates-io] (patched wgpu-hal, naga
# and libmimalloc-sys), and the game doesn't compile against the unpatched ones: repeat them here.
cargo check --tests \
  --config "patch.\"https://github.com/horizon-rust/horizon\".horizon-client.path=\"$horizon/crates/horizon-client\"" \
  --config "patch.crates-io.wgpu-hal.path=\"$horizon/third_party/wgpu-hal\"" \
  --config "patch.crates-io.naga.path=\"$horizon/third_party/naga\"" \
  --config "patch.crates-io.libmimalloc-sys.path=\"$horizon/third_party/libmimalloc-sys\""
