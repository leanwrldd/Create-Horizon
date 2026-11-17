"""The Rope Pulley, its Rope and its Pulley Magnet (module `pulley`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.ROPE_PULLEY` (`SharedProperties.stone`,
`MapColor.PODZOL`, `noOcclusion`, axe or pickaxe, `CStress.setImpact(4.0)`,
`HorizontalAxisKineticBlock`: `axis`), `ROPE` (`SoundType.WOOL`, `MapColor.COLOR_BROWN`,
climbable, no item) and `PULLEY_MAGNET` (`SharedProperties.stone`, climbable, no item),
`AllShapes.FOUR_VOXEL_POLE` and `PULLEY_MAGNET`, `AllPartialModels.ROPE_COIL`, `ROPE_HALF` and
`ROPE_HALF_MAGNET` and `recipe/crafting/kinetics/rope_pulley`. Waterlogging is left out, as
elsewhere. The behaviour is `native/api/src/machines/pulley.rs`.
"""

from gen_content import ANDESITE, AXES, PODZOL, block

BROWN = 26  # MapColor.COLOR_BROWN

# `AllShapes.FOUR_VOXEL_POLE` facing up, and `PULLEY_MAGNET`: shape(3, -3, 3, 13, 2, 13) with the
# pole (the part below the block's floor is left out of the cell).
_POLE = [6 / 16, 0, 6 / 16, 10 / 16, 1, 10 / 16]
_MAGNET = [[3 / 16, 0, 3 / 16, 13 / 16, 2 / 16, 13 / 16], _POLE]

_pulley = block({"axis": {"type": "enum", "values": ["x", "z"]}}, {"axis": "z"}, ANDESITE, PODZOL,
                render_shape="MODEL")
_pulley["properties"]["occludes"] = False  # `noOcclusion()`

# `Block.Properties.of()` with a wool sound: no hardness, no tool.
ROPE_PROPERTIES = {"hardness": 0.0, "explosion_resistance": 0.0, "requires_tool": False,
                   "sounds": "minecraft:white_wool"}
_rope = dict(block(None, None, ROPE_PROPERTIES, BROWN, {"default": [_POLE]}, render_shape="MODEL"),
             item=False)
_magnet = dict(block(None, None, ANDESITE, 0, {"default": _MAGNET}, render_shape="MODEL"),
               item=False)
for value in (_rope, _magnet):
    value["properties"]["occludes"] = False

BLOCKS = {"rope_pulley": _pulley, "rope": _rope, "pulley_magnet": _magnet}

MODELS = ["block/rope_pulley/rope_coil", "block/rope_pulley/rope_half",
          "block/rope_pulley/rope_half_magnet"]

TAB = ["rope_pulley"]

# `BlockTags.CLIMBABLE`.
TAGS = {"minecraft:block/climbable": ["create:rope", "create:pulley_magnet"]}

NAMES = {
    "block.create.rope_pulley": "Rope Pulley",
    "block.create.rope": "Rope",
    "block.create.pulley_magnet": "Pulley Magnet",
}

RECIPES = {
    # `crafting/kinetics/rope_pulley`, in Horizon's 26.3 form.
    "crafting/kinetics/rope_pulley": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["B", "C", "I"],
        "key": {"B": "create:andesite_casing", "C": "#minecraft:wool", "I": "#c:plates/iron"},
        "result": {"id": "create:rope_pulley", "count": 1},
    },
}
