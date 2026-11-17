"""The Clockwork Bearing: `create:clockwork_bearing` (module `clockwork_bearing`).

Create `mc1.21.1/dev` at 4148fd8: `AllBlocks.CLOCKWORK_BEARING` (`MapColor.TERRACOTTA_BROWN`, axe or pickaxe,
`BuilderTransformers.bearing("clockwork", "brass_gearbox")`: `SharedProperties.stone`, `noOcclusion`,
`CStress.setImpact(4.0)`), `ClockworkBearingBlock` (a `BearingBlock`: `facing`; waterlogging left out, as
elsewhere), `AllPartialModels.BEARING_TOP`, `recipe/crafting/kinetics/clockwork_bearing`.
The behaviour is `native/api/src/machines/clockwork_bearing.rs`.
"""

from gen_content import ANDESITE, FACING_STATE, block

TERRACOTTA_BROWN = 48  # MapColor.TERRACOTTA_BROWN

BLOCKS = {
    # The base is in the chunk mesh (`create:block/clockwork_bearing`); the block entity draws the shaft
    # out of the back, the turning top and the two hands.
    "clockwork_bearing": block(FACING_STATE, {"facing": "north"}, ANDESITE, TERRACOTTA_BROWN,
                               render_shape="MODEL"),
}

# The top is the Mechanical Bearing's (`block/bearing/top`, declared in `bearing.py`).

TAB = ["clockwork_bearing"]

NAMES = {
    "block.create.clockwork_bearing": "Clockwork Bearing",
    "create.contraptions.clockwork.clock_hands": "Clock Hand Arrangement",
    "create.contraptions.clockwork.hour_first": "Hour hand first",
    "create.contraptions.clockwork.minute_first": "Minute hand first",
    "create.contraptions.clockwork.hour_first_24": "24-Hour hand first",
}

RECIPES = {
    # `recipe/crafting/kinetics/clockwork_bearing`, in Horizon's 26.3 form.
    "crafting/kinetics/clockwork_bearing": {
        "type": "minecraft:crafting_shaped", "category": "misc",
        "pattern": ["B", "C", "S"],
        "key": {"B": "#minecraft:wooden_slabs", "C": "create:brass_casing", "S": "create:electron_tube"},
        "result": {"id": "create:clockwork_bearing", "count": 1},
    },
}
