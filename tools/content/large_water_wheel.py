"""The Large Water Wheel and its structure blocks (module `large_water_wheel`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.LARGE_WATER_WHEEL` (`SharedProperties.wooden`, `noOcclusion`,
`MapColor.DIRT`, `axeOrPickaxe`, `CStress.setCapacity(128.0)`, generator speed 4), the `extension`
property of `LargeWaterWheelBlock`, `AllBlocks.WATER_WHEEL_STRUCTURAL` (`create:water_wheel_structure`: no
item, invisible, `facing`; its loot table drops nothing), `recipe/crafting/kinetics/large_water_wheel`.
"""

from gen_content import AXIS_STATE, DIRT, FACING_STATE, WOODEN, block

BLOCKS = {
    # The wheel is drawn by its block entity: turning, with the extension when another wheel is on
    # its negative side.
    "large_water_wheel": block({**AXIS_STATE, "extension": {"type": "boolean"}},
                               {"axis": "y", "extension": "false"}, WOODEN, DIRT),
    # No item: the wheel fills these round itself. They draw nothing.
    "water_wheel_structure": dict(
        block(FACING_STATE, {"facing": "north"}, WOODEN, DIRT, render_shape="INVISIBLE"),
        item=False),
}

NAMES = {
    "block.create.large_water_wheel": "Large Water Wheel",
    "block.create.water_wheel_structure": "Large Water Wheel",
}

# `AllPartialModels.LARGE_WATER_WHEEL` and `LARGE_WATER_WHEEL_EXTENSION`.
MODELS = ["create:block/large_water_wheel/block", "create:block/large_water_wheel/block_extension"]

RECIPES = {
    # `crafting/kinetics/large_water_wheel`.
    "large_water_wheel": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["SSS", "SCS", "SSS"],
        "key": {"S": "#minecraft:planks", "C": "create:water_wheel"},
        "result": {"id": "create:large_water_wheel"},
    },
}

TAB = ["large_water_wheel"]
