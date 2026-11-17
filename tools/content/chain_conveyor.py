"""The Chain Conveyor: `create:chain_conveyor`.

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.CHAIN_CONVEYOR` (`SharedProperties.stone`, `noOcclusion`,
`MapColor.PODZOL`, `axeOrPickaxe`, `CStress.setImpact(1)`), `ChainConveyorBlock` (a `KineticBlock` on the
Y axis, no state), the blockstate (the one model `block/chain_conveyor/block`), the partial models the
block entity draws (`AllPartialModels.CHAIN_CONVEYOR_SHAFT`, `_WHEEL`, `_GUARD`, `.obj` meshes read from
the jar), `recipe/crafting/kinetics/chain_conveyor` (two from one). The behaviour is
`native/api/src/machines/chain_conveyor.rs` (known differences at its top).
"""
from gen_content import ANDESITE, PODZOL, block

BLOCKS = {
    # `AllShapes.CHAIN_CONVEYOR_INTERACTION` reaches past the block; the shape here is the block.
    "chain_conveyor": block(None, None, ANDESITE, PODZOL, render_shape="MODEL"),
}

MODELS = ["block/chain_conveyor/shaft", "block/chain_conveyor/wheel", "block/chain_conveyor/guard"]

NAMES = {"block.create.chain_conveyor": "Chain Conveyor"}
TAB = ["chain_conveyor"]

RECIPES = {
    "crafting/kinetics/chain_conveyor": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": [" C ", "CAC", " C "],
        "key": {"A": "create:large_cogwheel", "C": "create:andesite_casing"},
        "result": {"id": "create:chain_conveyor", "count": 2},
    },
}
