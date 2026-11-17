"""The Hose Pulley: `create:hose_pulley`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.HOSE_PULLEY` (`SharedProperties.copperMetal`: copper
block's 3.0 and 6.0 and its sound, `noOcclusion`, `pickaxeOnly`, `CStress.setImpact(4.0)`,
`BlockStateGen.horizontalBlockProvider(true)`: `facing`), `HosePulleyBlock`
(`HORIZONTAL_FACING`), `AllPartialModels.HOSE_COIL`, `HOSE`, `HOSE_MAGNET`, `HOSE_HALF` and
`HOSE_HALF_MAGNET`, and `recipe/crafting/kinetics/hose_pulley`. The block is in the chunk mesh
(Create's own `block/hose_pulley/block` per facing); the block entity draws the shaft, the coil,
the hose and its end piece. The behaviour is `native/api/src/machines/hose_pulley.rs`.
"""

from gen_content import HORIZONTAL_FACING_STATE, block

# `Blocks.COPPER_BLOCK`: `MapColor.COLOR_ORANGE`.
ORANGE = 15
COPPER = {"hardness": 3.0, "explosion_resistance": 6.0, "requires_tool": True,
          "sounds": "minecraft:copper_block"}

_hose_pulley = block(HORIZONTAL_FACING_STATE, {"facing": "north"}, COPPER, ORANGE,
                     render_shape="MODEL")
_hose_pulley["properties"]["occludes"] = False  # `noOcclusion()`

BLOCKS = {"hose_pulley": _hose_pulley}
PICKAXE_ONLY = {"hose_pulley"}
NAMES = {"block.create.hose_pulley": "Hose Pulley"}
TAB = ["hose_pulley"]

# `AllPartialModels.HOSE_COIL`, `HOSE`, `HOSE_MAGNET`, `HOSE_HALF`, `HOSE_HALF_MAGNET`.
MODELS = [
    "block/hose_pulley/hose_coil",
    "block/hose_pulley/rope",
    "block/hose_pulley/pulley_magnet",
    "block/hose_pulley/rope_half",
    "block/hose_pulley/rope_half_magnet",
]

RECIPES = {
    # `crafting/kinetics/hose_pulley`, in Horizon's 26.3 form.
    "crafting/kinetics/hose_pulley": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["B", "C", "I"],
        "key": {"B": "create:copper_casing", "C": "minecraft:dried_kelp_block", "I": "#c:plates/copper"},
        "result": {"id": "create:hose_pulley", "count": 1},
    },
}
