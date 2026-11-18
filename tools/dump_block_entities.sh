#!/usr/bin/env bash
# Asks the modules which blocks have a block entity and writes tools/block_entities.txt, which
# tools/gen_parts.py turns into mod/src/parts_gen.rs. Run it after a module starts or stops giving a block an entity,
# then `python tools/gen_parts.py`.
#   tools/dump_block_entities.sh [path to the Horizon checkout the SDK was built from]
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
if [ -n "${1:-}" ]; then
  horizon="$(cd "$1" && pwd)"
else
  dir="$root"; horizon=""
  while [ "$dir" != "/" ] && [ -z "$horizon" ]; do
    dir="$(dirname "$dir")"
    [ -d "$dir/horizon-latest" ] && horizon="$dir/horizon-latest"
  done
  [ -n "$horizon" ] || { echo "no horizon-latest checkout found; pass it as the argument" >&2; exit 1; }
fi
if command -v cygpath >/dev/null 2>&1; then horizon="$(cygpath -m "$horizon")"; fi
work="$root/target/check-dump"
rm -rf "$work/mod"; mkdir -p "$work"
cp -r "$root/mod" "$work/mod"
find "$work/mod" -name Cargo.lock -delete
python "$root/tools/gen_modules.py" "$work/mod"
sed -i "s#path = \".horizon/sdk/api/crates/horizon-mod-api\"#path = \"$horizon/crates/horizon-mod-api\"#" "$work/mod/Cargo.toml"
cp "$horizon/Cargo.lock" "$work/mod/Cargo.lock"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target/package-check/target}"
out="$root/tools/block_entities.txt"
cd "$work/mod"
CREATE_BLOCK_ENTITIES_OUT="$(cygpath -m "$out" 2>/dev/null || echo "$out")" cargo test --lib dump_block_entities -- --ignored
echo "wrote $out ($(wc -l < "$out") blocks)"
