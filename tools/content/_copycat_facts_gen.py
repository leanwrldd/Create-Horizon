"""One-off generator for the block facts the Copycats read (not a plugin: the leading underscore
keeps tools/gen_content.py from loading it). Writes native/kinetics/src/rules/copycat_facts.rs.

    python tools/content/_copycat_facts_gen.py [prepared game data dir]

`CopycatBlock.getAcceptedBlockState` asks a block for its shape, its collision, whether it is an
`EntityBlock` or a `StairBlock`, and which of `FACING`, `HORIZONTAL_FACING`, `AXIS` and
`HORIZONTAL_AXIS` it has: Horizon's `Host` answers none of that for a block that isn't placed, so
vanilla's answers are baked in from the prepared game data (`facts.json` names each state and its
properties, `gameplay-facts.json` the outline, collision and block entity, same order). The blocks of
this mod that are plain full cubes (a declaration on a vanilla cube template, with no shape of its
own and no block entity) are added from this mod's declarations. Rerun it when the game version
changes or a block is added.
"""

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
sys.path.insert(0, str(ROOT / "tools"))
DEFAULT_DATA = Path.home() / "AppData/Roaming/horizon/.horizon-cache/mod-content/5cce03fe38433e65"
OUT = ROOT / "native/kinetics/src/rules/copycat_facts.rs"

FULL = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0]
# `data/create/tags/block/copycat_deny.json`: cauldrons, saplings and climbable blocks, which the
# full-cube test would mostly turn away anyway.
DENY_NAMES = ("cauldron", "sapling")
CLIMBABLE = {"minecraft:ladder", "minecraft:vine", "minecraft:scaffolding", "minecraft:twisting_vines",
             "minecraft:twisting_vines_plant", "minecraft:weeping_vines", "minecraft:weeping_vines_plant",
             "minecraft:cave_vines", "minecraft:cave_vines_plant"}
# `data/create/tags/block/copycat_allow.json`.
ALLOW = {"minecraft:barrel"}


def bounds(boxes):
    if not boxes:
        return None
    return [min(b[i] for b in boxes) for i in range(3)] + [max(b[i] for b in boxes) for i in range(3, 6)]


def main():
    data = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_DATA
    facts = json.loads((data / "facts.json").read_text(encoding="utf-8"))["states"]
    gameplay = json.loads((data / "gameplay-facts.json").read_text(encoding="utf-8"))["states"]
    assert len(facts) == len(gameplay)
    values = {}  # block -> property -> set of values
    first = {}
    for state, extra in zip(facts, gameplay):
        name = state[0]
        props = state[4].get("properties", {})
        for key, value in props.items():
            values.setdefault(name, {}).setdefault(key, set()).add(value)
        first.setdefault(name, extra)

    accepted = set()
    for name, extra in first.items():
        if not name.startswith("minecraft:") or not extra.get("item") or extra["item"] == "minecraft:air":
            continue
        if extra["item"] != name:
            continue  # the item isn't the block's own (a bucket, a seed)
        if name in ALLOW:
            accepted.add(name)
            continue
        if extra["block_entity"] or name.endswith("_stairs") or any(d in name for d in DENY_NAMES):
            continue
        if name in CLIMBABLE or extra["render_shape"] == "INVISIBLE":
            continue
        if bounds(extra["outline"]) == FULL and extra["full_collision"]:
            accepted.add(name)

    # This mod's plain cubes: a declaration on an accepted template with no shape, state or block
    # entity of its own (but `axis`, which a pillar has).
    import gen_content

    collected = gen_content.collect()
    for name, decl in collected.blocks.items():
        template = decl.get("template", "")
        props = decl.get("properties", {})
        if not decl.get("item", True) or "shapes" in decl or props.get("render_shape") == "ENTITYBLOCK_ANIMATED":
            continue
        if set(decl.get("state_properties", {})) - {"axis"}:
            continue
        if template == "minecraft:glass" and "render_shape" in props:
            continue
        if template in accepted or template == "minecraft:glass":
            accepted.add(f"create:{name}")
            if "axis" in decl.get("state_properties", {}) or template == "minecraft:quartz_pillar":
                values.setdefault(f"create:{name}", {})["axis"] = {"x", "y", "z"}

    def kind(key, wanted):
        return sorted(n for n in accepted if values.get(n, {}).get(key) == wanted)

    facing6 = kind("facing", {"north", "south", "east", "west", "up", "down"})
    facing4 = kind("facing", {"north", "south", "east", "west"})
    axis3 = kind("axis", {"x", "y", "z"})
    axis2 = kind("axis", {"x", "z"})
    lit = sorted(n for n in accepted if values.get(n, {}).get("lit") == {"true", "false"})
    trapdoors = sorted(n for n in values if n.endswith("_trapdoor")
                       and {"half", "open", "facing"} <= set(values[n]))
    bars = sorted(n for n in first if n.startswith("minecraft:") and n.endswith("_bars")
                  or n == "minecraft:iron_bars") + sorted(
        f"create:{n}" for n in collected.blocks if n.endswith("_bars") and n != "copycat_bars")

    def rows(names):
        return "".join(f'    "{name}",\n' for name in sorted(set(names)))

    OUT.write_text(f'''//! Generated by tools/content/_copycat_facts_gen.py: do not edit by hand.
//!
//! What `CopycatBlock.getAcceptedBlockState` asks a block that Horizon's `Host` cannot answer for a
//! block that isn't placed, by block id, from the prepared game data and this mod's declarations.

/// Blocks a Copycat takes as its material: a full outline and collision, no block entity, not a
/// stair, not denied (`copycat_deny`), plus `copycat_allow`.
pub const ACCEPTED: &[&str] = &[
{rows(accepted)}];

/// Of those, the blocks with `BlockStateProperties.FACING` (six values).
pub const FACING: &[&str] = &[
{rows(facing6)}];

/// `HORIZONTAL_FACING` (four values, named `facing`).
pub const HORIZONTAL_FACING: &[&str] = &[
{rows(facing4)}];

/// `BlockStateProperties.AXIS` (x, y, z).
pub const AXIS: &[&str] = &[
{rows(axis3)}];

/// `HORIZONTAL_AXIS` (x, z).
pub const HORIZONTAL_AXIS: &[&str] = &[
{rows(axis2)}];

/// `BlockStateProperties.LIT`, which a click cycles.
pub const LIT: &[&str] = &[
{rows(lit)}];

/// `CopycatSpecialCases.isTrapdoorMaterial`: trapdoors with `half`, `open` and `facing`.
pub const TRAPDOORS: &[&str] = &[
{rows(trapdoors)}];

/// `CopycatSpecialCases.isBarsMaterial`: `IronBarsBlock`s that aren't glass panes.
pub const BARS: &[&str] = &[
{rows(bars)}];
''', encoding="utf-8", newline="\n")
    print(f"{len(accepted)} accepted, {len(facing6)} facing, {len(facing4)} horizontal facing, "
          f"{len(axis3)} axis, {len(axis2)} horizontal axis, {len(trapdoors)} trapdoors, {len(bars)} bars")


main()
