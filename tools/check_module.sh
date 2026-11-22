#!/usr/bin/env bash
# Tests and type-checks the package as one agent sees it: its own modules plus everything finished,
# without the modules other agents are still writing.
#   tools/check_module.sh <owner> [path to the Horizon checkout the SDK was built from]
# tools/wip.txt lists who is writing what: `<owner>: <module> <module> ...`. A module name covers
# mod/src/machines/<name>.rs and <name>_*.rs, and the same under mod/src/kinetics/src/rules/. Other owners'
# files are left out of a private copy under target/check-<owner>/, the module lists are regenerated
# there, and then it runs
#   cargo test    on mod/src/kinetics (the rules)
#   cargo check   on the package against the checkout's mod API (the hooks)
# With an owner of `all`, nothing is left out: the whole tree as it stands.
#
# The launcher takes only `horizon-mod-api = { path = ".horizon/sdk/api/crates/horizon-mod-api" }`, which a plain
# cargo can't resolve here: the private copy swaps in the checkout's crate and the checkout's Cargo.lock. The real
# mod/Cargo.toml is never touched.
set -euo pipefail
owner="${1:?usage: tools/check_module.sh <owner> [horizon checkout]}"
root="$(cd "$(dirname "$0")/.." && pwd)"
if [ -n "${2:-}" ]; then
  horizon="$(cd "$2" && pwd)"
else
  # the checkout is a sibling of the repository (or of the main checkout, from a worktree)
  dir="$root"
  horizon=""
  while [ "$dir" != "/" ] && [ -z "$horizon" ]; do
    dir="$(dirname "$dir")"
    [ -d "$dir/horizon-latest" ] && horizon="$dir/horizon-latest"
  done
  # then $HORIZON_LATEST, then where it lived before the repository moved (~/Claude/Projects)
  [ -n "$horizon" ] || { [ -n "${HORIZON_LATEST:-}" ] && [ -d "$HORIZON_LATEST" ] && horizon="$HORIZON_LATEST"; }
  [ -n "$horizon" ] || { [ -d "$HOME/Claude/Projects/horizon-latest" ] && horizon="$HOME/Claude/Projects/horizon-latest"; }
  [ -n "$horizon" ] || { echo "no horizon-latest checkout found; pass it as the second argument or set HORIZON_LATEST" >&2; exit 1; }
fi
if command -v cygpath >/dev/null 2>&1; then horizon="$(cygpath -m "$horizon")"; fi
work="$root/target/check-$owner"
rm -rf "$work/mod"
mkdir -p "$work"
cp -r "$root/mod" "$work/mod"
find "$work/mod" -name Cargo.lock -delete
find "$work/mod" -name target -type d -prune -exec rm -rf {} +
if [ "$owner" != all ] && [ -f "$root/tools/wip.txt" ]; then
  while IFS= read -r line; do
    case "$line" in ''|'#'*) continue;; esac
    who="${line%%:*}"
    [ "$who" = "$owner" ] && continue
    for name in ${line#*:}; do
      rm -f "$work/mod/src/machines/$name.rs" "$work/mod/src/machines/${name}"_*.rs \
            "$work/mod/src/kinetics/src/rules/$name.rs" "$work/mod/src/kinetics/src/rules/${name}"_*.rs
    done
  done < "$root/tools/wip.txt"
fi
python "$root/tools/gen_modules.py" "$work/mod"
# One target directory for every owner: the mod API is compiled once.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target/package-check/target}"
echo "== cargo test: mod/src/kinetics =="
(cd "$work/mod/src/kinetics" && cargo test 2>&1 | grep -v '^\s*Compiling\|^\s*Blocking' | tail -n 40)
rm -f "$work/mod/src/kinetics/Cargo.lock"
# The path swap and the checkout's own lock, so dependencies resolve to the versions the game is built with.
sed -i "s#path = \".horizon/sdk/api/crates/horizon-mod-api\"#path = \"$horizon/crates/horizon-mod-api\"#" "$work/mod/Cargo.toml"
cp "$horizon/Cargo.lock" "$work/mod/Cargo.lock"
cd "$work/mod"
echo "== cargo check: the package =="
cargo check --tests
