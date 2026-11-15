"""The Mechanical Piston, the Sticky Mechanical Piston, the Piston Extension Pole and the
Mechanical Piston Head (module `piston`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.MECHANICAL_PISTON` and `STICKY_MECHANICAL_PISTON`
(`SharedProperties.stone`, `MapColor.PODZOL`, axe or pickaxe, `noOcclusion`, `CStress.setImpact(4.0)`,
`BuilderTransformers.mechanicalPiston`), `PISTON_EXTENSION_POLE` (`Blocks.PISTON_HEAD`'s
properties with the scaffolding sound, `MapColor.DIRT`) and `MECHANICAL_PISTON_HEAD` (no item,
drops a pole), `MechanicalPistonBlock` (`facing`, `axis_along_first`, `state`),
`PistonExtensionPoleBlock` (`facing`), `MechanicalPistonHeadBlock` (`facing`, `type`; waterlogging
is left out, as elsewhere), `AllShapes` (`MECHANICAL_PISTON`, `MECHANICAL_PISTON_EXTENDED`,
`MECHANICAL_PISTON_HEAD`, `FOUR_VOXEL_POLE`), and the recipes under
`data/create/recipe/crafting/kinetics/`.
"""

from gen_content import ANDESITE, DIRT, FACING_STATE, FACINGS, PODZOL, block, facing_shapes

PISTON_STATES = {
    **FACING_STATE,
    "axis_along_first": {"type": "boolean"},
    "state": {"type": "enum", "values": ["retracted", "moving", "extended"]},
}
PISTON_DEFAULT = {"facing": "north", "axis_along_first": "false", "state": "retracted"}

# `AllShapes.CASING_12PX`, written facing up; `FOUR_VOXEL_POLE` is shape(6, 0, 6, 10, 16, 10).
CASING = [0, 0, 0, 1, 12 / 16, 1]
POLE = [6 / 16, 0, 6 / 16, 10 / 16, 1, 10 / 16]
# `MechanicalPistonBlock.getShape`: a block retracted, the casing while moving, the casing and
# the pole through it extended (`MECHANICAL_PISTON_EXTENDED`).
_casing = facing_shapes([CASING])["values"]
_extended = facing_shapes([CASING, POLE])["values"]
PISTON_SHAPES = {
    "properties": ["facing", "state"],
    "values": {
        **{f"{f},retracted": [[0, 0, 0, 1, 1, 1]] for f in FACINGS},
        **{f"{f},moving": _casing[f] for f in FACINGS},
        **{f"{f},extended": _extended[f] for f in FACINGS},
    },
}

# `PISTON_HEAD` (a vanilla short head facing up): the plate on the face and the arm behind it.
HEAD_SHAPES = facing_shapes([[0, 12 / 16, 0, 1, 1, 1], [6 / 16, 0, 6 / 16, 10 / 16, 12 / 16, 10 / 16]])

# `Blocks.PISTON_HEAD.strength(1.5f)`; the pole's `SoundType.SCAFFOLDING`.
PISTON_HEAD = {"hardness": 1.5, "explosion_resistance": 1.5, "requires_tool": False,
               "sounds": "minecraft:stone"}
POLE_PROPERTIES = dict(PISTON_HEAD, sounds="minecraft:scaffolding")

BLOCKS = {
    "mechanical_piston": block(PISTON_STATES, PISTON_DEFAULT, ANDESITE, PODZOL, PISTON_SHAPES,
                               render_shape="MODEL"),
    "sticky_mechanical_piston": block(PISTON_STATES, PISTON_DEFAULT, ANDESITE, PODZOL, PISTON_SHAPES,
                                      render_shape="MODEL"),
    "piston_extension_pole": block(FACING_STATE, {"facing": "up"}, POLE_PROPERTIES, DIRT,
                                   facing_shapes([POLE]), render_shape="MODEL"),
    # No item: `.register()` without `.item()`.
    "mechanical_piston_head": dict(
        block({**FACING_STATE, "type": {"type": "enum", "values": ["normal", "sticky"]}},
              {"facing": "north", "type": "normal"}, PISTON_HEAD, DIRT,
              HEAD_SHAPES, render_shape="MODEL"),
        item=False),
}
# `axeOrPickaxe()`: the default tags (both tools).

# The head drops a pole (`p.dropOther(b, PISTON_EXTENSION_POLE.get())`).
LOOT = {
    "mechanical_piston_head": [{
        "rolls": 1,
        "entries": [{"type": "minecraft:item", "name": "create:piston_extension_pole"}],
        "condition": {"type": "minecraft:survives_explosion"},
    }],
}

TAB = ["mechanical_piston", "sticky_mechanical_piston", "piston_extension_pole"]

NAMES = {
    "block.create.mechanical_piston": "Mechanical Piston",
    "block.create.sticky_mechanical_piston": "Sticky Mechanical Piston",
    "block.create.piston_extension_pole": "Piston Extension Pole",
    "block.create.mechanical_piston_head": "Mechanical Piston Head",
}

RECIPES = {
    # `crafting/kinetics/mechanical_piston`, `sticky_mechanical_piston`, `piston_extension_pole`
    # in Horizon's 26.3 form.
    "crafting/kinetics/mechanical_piston": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["B", "C", "I"],
        "key": {"B": "#minecraft:wooden_slabs", "C": "create:andesite_casing",
                "I": "create:piston_extension_pole"},
        "result": {"id": "create:mechanical_piston", "count": 1},
    },
    "crafting/kinetics/sticky_mechanical_piston": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["S", "P"],
        "key": {"S": "#c:slimeballs", "P": "create:mechanical_piston"},
        "result": {"id": "create:sticky_mechanical_piston", "count": 1},
    },
    "crafting/kinetics/piston_extension_pole": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["P", "A", "P"],
        "key": {"P": "#minecraft:planks", "A": "create:andesite_alloy"},
        "result": {"id": "create:piston_extension_pole", "count": 8},
    },
}

# NeoForge's `c:slimeballs` (`Tags.Items.SLIMEBALLS`) of vanilla's items.
TAGS = {"c:item/slimeballs": ["minecraft:slime_ball"]}
